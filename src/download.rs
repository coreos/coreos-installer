// Copyright 2019 CoreOS, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::{anyhow, bail, Context, Result};
use byte_unit::Byte;
use nix::unistd::isatty;
use std::fs::{remove_file, File, OpenOptions};
use std::io::{self, copy, stderr, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::num::{NonZeroU32, NonZeroU64};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::result;
use std::time::{Duration, Instant};

use crate::blockdev::{detect_formatted_sector_size, get_gpt_size, SavedPartitions};
use crate::cmdline::*;
use crate::io::*;
use crate::source::*;
use crate::verify::*;

// Download all artifacts for an image and verify their signatures.
pub fn download(config: &DownloadConfig) -> Result<()> {
    // walk sources
    let mut sources = config.location.sources()?;
    if sources.is_empty() {
        bail!("no artifacts found");
    }
    for mut source in sources.iter_mut() {
        // set up image source
        if source.signature.is_none() {
            if config.insecure {
                eprintln!("Signature not found; skipping verification as requested");
            } else {
                bail!("--insecure not specified and signature not found");
            }
        }

        // calculate paths
        let filename = if config.decompress {
            // Drop any compression suffix.  Hacky.
            source
                .filename
                .trim_end_matches(".gz")
                .trim_end_matches(".xz")
                .to_string()
        } else {
            source.filename.to_string()
        };
        let mut path = PathBuf::new();
        path.push(&config.directory);
        path.push(&filename);
        let sig_path = path.with_file_name(format!("{}.sig", &filename));

        // check existing image and signature; don't redownload if OK
        // If we decompressed last time, the call will fail because we can't
        // check the old signature.  If we didn't decompress last time but are
        // decompressing this time, we're not smart enough to decompress the
        // existing file.
        if !config.decompress && check_image_and_sig(&source, &path, &sig_path).is_ok() {
            // report the output file path and keep going
            println!("{}", path.display());
            continue;
        }

        // write the image and signature
        if let Err(err) = write_image_and_sig(&mut source, &path, &sig_path, config.decompress) {
            // delete output files, which may not have been created yet
            let _ = remove_file(&path);
            let _ = remove_file(&sig_path);

            // fail
            return Err(err);
        }

        // report the output file path
        println!("{}", path.display());
    }

    Ok(())
}

// Check an existing image and signature for validity.  The image cannot
// have been decompressed after downloading.  Return an error if invalid for
// any reason.
fn check_image_and_sig(source: &ImageSource, path: &Path, sig_path: &Path) -> Result<()> {
    // ensure we have something to check
    if source.signature.is_none() {
        bail!("no signature available; can't check existing file");
    }
    let signature = source.signature.as_ref().unwrap();

    // compare signature to expectation
    let mut sig_file = OpenOptions::new()
        .read(true)
        .open(sig_path)
        .with_context(|| format!("opening {}", sig_path.display()))?;
    let mut buf = Vec::new();
    sig_file
        .read_to_end(&mut buf)
        .with_context(|| format!("reading {}", sig_path.display()))?;
    if &buf != signature {
        bail!("signature file doesn't match source");
    }

    // open image file
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;

    // perform GPG verification
    GpgReader::new(&mut file, signature)?.consume()?;

    Ok(())
}

/// Copy the image to disk, and also the signature if appropriate.
fn write_image_and_sig(
    source: &mut ImageSource,
    path: &Path,
    sig_path: &Path,
    decompress: bool,
) -> Result<()> {
    // open output
    let mut dest = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;

    // download and verify image
    // don't check sector size
    write_image(
        source,
        &mut dest,
        path,
        image_copy_default,
        decompress,
        None,
        None,
    )?;

    // write signature, if relevant
    if let (false, Some(signature)) = (decompress, source.signature.as_ref()) {
        let mut sig_dest = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(sig_path)
            .with_context(|| format!("opening {}", sig_path.display()))?;
        sig_dest
            .write_all(signature)
            .context("writing signature data")?;
    }

    Ok(())
}

/// Copy the image to disk and verify its signature.
pub fn write_image<F>(
    source: &mut ImageSource,
    dest: &mut File,
    dest_path: &Path,
    image_copy: F,
    decompress: bool,
    saved: Option<&SavedPartitions>,
    expected_sector_size: Option<NonZeroU32>,
) -> Result<()>
where
    F: FnOnce(&[u8], &mut dyn Read, &mut File, &Path, Option<&SavedPartitions>) -> Result<()>,
{
    // wrap source for GPG verification
    let mut verify_reader: Box<dyn Read> = {
        if let Some(signature) = source.signature.as_ref() {
            Box::new(GpgReader::new(&mut source.reader, signature)?)
        } else {
            Box::new(&mut source.reader)
        }
    };

    // wrap again for progress reporting
    let mut progress_reader = ProgressReader::new(
        &mut verify_reader,
        source.length_hint,
        &source.artifact_type,
    );

    // Wrap in a BufReader so DecompressReader can peek at the first few
    // bytes for format sniffing, and to amortize read overhead.  Don't
    // trust the content-type since the server may not be configured
    // correctly, or the file might be local.  Then wrap in a
    // DecompressReader for decompression.
    let mut buf_reader = BufReader::with_capacity(BUFFER_SIZE, &mut progress_reader);
    let decompress_reader: Box<dyn Read> = if decompress {
        Box::new(DecompressReader::new(&mut buf_reader)?)
    } else {
        Box::new(buf_reader)
    };

    // Wrap again for limit checking.
    let byte_limit = saved.map(|saved| saved.get_offset()).transpose()?.flatten();
    let mut limit_reader: Box<dyn Read> = match byte_limit {
        None => Box::new(decompress_reader),
        Some((limit, conflict)) => Box::new(LimitReader::new(decompress_reader, limit, conflict)),
    };

    // Read the first MiB of input and, if requested, check it against the
    // image's formatted sector size.
    let mut first_mb = [0u8; 1024 * 1024];
    limit_reader
        .read_exact(&mut first_mb)
        .context("decoding first MiB of image")?;
    // Were we asked to check sector size?
    if let Some(expected) = expected_sector_size {
        // Can we derive one from source data?
        if let Some(actual) = detect_formatted_sector_size(&first_mb) {
            // Do they match?
            if expected != actual {
                bail!(
                    "source has sector size {} but destination has sector size {}",
                    actual.get(),
                    expected.get()
                );
            }
        }
    }

    // call the callback to copy the image
    image_copy(&first_mb, &mut limit_reader, dest, dest_path, saved)?;

    // finish I/O before closing the progress bar
    dest.sync_all().context("syncing data to disk")?;

    Ok(())
}

pub fn image_copy_default(
    first_mb: &[u8],
    source: &mut dyn Read,
    dest: &mut File,
    _dest_path: &Path,
    saved: Option<&SavedPartitions>,
) -> Result<()> {
    // Don't write the first MiB yet.  This ensures that the disk image
    // can't be used accidentally before its GPG signature is verified.  If
    // this is a real disk, write the saved partitions (so they don't get
    // lost if we crash), and otherwise write zeroes.
    match saved {
        Some(saved) => {
            saved
                .overwrite(dest)
                .context("overwriting disk partition table")?;
            dest.seek(SeekFrom::Start(1024 * 1024))
                .context("seeking disk")?;
        }
        None => dest
            .write_all(&[0u8; 1024 * 1024])
            .context("clearing first MiB of disk")?,
    };
    dest.sync_all().context("syncing data to disk")?;

    // do the rest of the copy
    // This physically writes any runs of zeroes, rather than sparsifying,
    // but sparsifying is unsafe.  We can't trust that all runs of zeroes in
    // the image represent unallocated blocks, so we must ensure that zero
    // blocks are actually stored as zeroes to avoid image corruption.
    // Discard is insufficient for this: even if our discard request
    // succeeds, discard is not guaranteed to zero blocks (see kernel
    // commits 98262f2762f0 and 48920ff2a5a9).  Ideally we could use
    // BLKZEROOUT to perform hardware-accelerated zeroing and then
    // sparse-copy the image, falling back to non-sparse copy if hardware
    // acceleration is unavailable.  But BLKZEROOUT doesn't support
    // BLKDEV_ZERO_NOFALLBACK, so we'd risk gigabytes of redundant I/O.
    //
    // Amortize write overhead.  The decompressor will produce bytes in
    // whatever chunk size it chooses.
    let mut buf_dest = BufWriter::with_capacity(BUFFER_SIZE, dest);
    copy(source, &mut buf_dest).context("decoding and writing image")?;
    // we can't retain the original error via context() because of lifetime
    // issues
    let dest = buf_dest
        .into_inner()
        .map_err(|_| anyhow!("flushing data to disk"))?;

    // verify_reader has now checked the signature, so fill in the first MiB
    let offset = match saved {
        Some(saved) if saved.is_saved() => {
            // copy MBR
            dest.seek(SeekFrom::Start(0))
                .context("seeking disk to MBR")?;
            dest.write_all(&first_mb[0..saved.get_sector_size() as usize])
                .context("writing MBR")?;

            // write merged GPT
            let mut cursor = Cursor::new(first_mb);
            saved
                .merge(&mut cursor, dest)
                .context("writing updated GPT")?;

            // copy all remaining bytes from first_mb (probably not
            // important but can't hurt)
            get_gpt_size(dest).context("getting GPT size")?
        }
        _ => {
            // copy all of first_mb
            0
        }
    };
    // do the copy
    dest.seek(SeekFrom::Start(offset))
        .with_context(|| format!("seeking disk to offset {}", offset))?;
    dest.write_all(&first_mb[offset as usize..first_mb.len()])
        .context("writing first MiB of disk")?;

    Ok(())
}

pub fn download_to_tempfile(url: &str, retries: FetchRetries) -> Result<File> {
    let mut f = tempfile::tempfile()?;

    let client = new_http_client()?;
    let mut resp = http_get(client, url, retries)?;

    let mut writer = BufWriter::with_capacity(BUFFER_SIZE, &mut f);
    copy(
        &mut BufReader::with_capacity(BUFFER_SIZE, &mut resp),
        &mut writer,
    )
    .with_context(|| format!("couldn't copy '{}'", url))?;
    writer
        .flush()
        .with_context(|| format!("couldn't write '{}' to disk", url))?;
    drop(writer);
    f.seek(SeekFrom::Start(0))
        .with_context(|| format!("rewinding file for '{}'", url))?;

    Ok(f)
}

struct ProgressReader<'a, R: Read> {
    source: R,
    length: Option<(NonZeroU64, String)>,
    artifact_type: &'a str,

    position: u64,
    last_report: Instant,

    tty: bool,
    prologue: &'static str,
    epilogue: &'static str,
}

impl<'a, R: Read> ProgressReader<'a, R> {
    fn new(source: R, length: Option<u64>, artifact_type: &'a str) -> Self {
        let tty = isatty(stderr().as_raw_fd()).unwrap_or_else(|e| {
            eprintln!("checking if stderr is a TTY: {}", e);
            false
        });
        // disable percentage reporting for zero-length files to avoid
        // division by zero
        let length = length.map(NonZeroU64::new).flatten();
        ProgressReader {
            source,
            length: length.map(|l| (l, Self::format_bytes(l.get()))),
            artifact_type,

            position: 0,
            last_report: Instant::now(),

            tty,
            // If stderr is a tty, draw a status line that updates itself in
            // place.  The prologue leaves a place for the cursor to rest
            // between updates.  The epilogue writes three spaces to cover
            // the switch from e.g.  1000 KiB to 1 MiB, and then uses CR to
            // return to the start of the line.
            //
            // Otherwise, stderr is being read by another process, e.g.
            // journald, and fanciness may confuse it.  Just log regular
            // lines.
            prologue: if tty { "> " } else { "" },
            epilogue: if tty { "   \r" } else { "\n" },
        }
    }

    /// Format a size in bytes.
    fn format_bytes(count: u64) -> String {
        Byte::from_bytes(count.into())
            .get_appropriate_unit(true)
            .format(1)
    }
}

impl<'a, R: Read> Read for ProgressReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> result::Result<usize, io::Error> {
        let count = self.source.read(buf)?;
        self.position += count as u64;
        if self.last_report.elapsed() >= Duration::from_secs(1)
            || self.length.as_ref().map(|(l, _)| l.get()) == Some(self.position)
        {
            self.last_report = Instant::now();
            match self.length {
                Some((length, ref length_str)) => eprint!(
                    "{}Read {} {}/{} ({}%){}",
                    self.prologue,
                    self.artifact_type,
                    Self::format_bytes(self.position),
                    length_str,
                    100 * self.position / length.get(),
                    self.epilogue
                ),
                None => eprint!(
                    "{}Read {} {}{}",
                    self.prologue,
                    self.artifact_type,
                    Self::format_bytes(self.position),
                    self.epilogue
                ),
            }
            let _ = std::io::stdout().flush();
        }
        Ok(count)
    }
}

impl<'a, R: Read> Drop for ProgressReader<'a, R> {
    fn drop(&mut self) {
        // if we reported progress using CRs, log final newline
        if self.tty {
            eprintln!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gptman::{GPTPartitionEntry, GPT};
    use std::io::{Seek, SeekFrom};
    use uuid::Uuid;

    #[test]
    fn test_write_image_limit() {
        // source must be partitioned if we're saving partitions
        let (mut source, source_path) = tempfile::Builder::new()
            .prefix("coreos-installer-")
            .tempfile()
            .unwrap()
            .into_parts();
        source.set_len(6 * 1024 * 1024).unwrap();
        partition(&mut source, None);

        let (mut dest, dest_path) = tempfile::Builder::new()
            .prefix("coreos-installer-")
            .tempfile()
            .unwrap()
            .into_parts();
        dest.set_len(8 * 1024 * 1024).unwrap();
        partition(&mut dest, Some(4));
        let saved = SavedPartitions::new_from_file(
            &mut dest,
            512,
            &vec![PartitionFilter::Label(glob::Pattern::new("*").unwrap())],
        )
        .unwrap();
        assert!(saved.is_saved());
        let offset = 4 * 1024 * 1024;
        let precious = "hello world";
        dest.seek(SeekFrom::Start(offset)).unwrap();
        dest.write_all(precious.as_bytes()).unwrap();
        dest.seek(SeekFrom::Start(0)).unwrap();

        let err = write_image(
            &mut FileLocation::new(source_path.to_str().unwrap())
                .sources()
                .unwrap()
                .remove(0),
            &mut dest,
            &dest_path,
            image_copy_default,
            false,
            Some(&saved),
            None,
        )
        .unwrap_err();
        assert!(
            format!("{:#}", err).contains("collision with partition"),
            "incorrect error: {:#}",
            err
        );

        dest.seek(SeekFrom::Start(offset)).unwrap();
        let mut buf = vec![0u8; precious.len()];
        dest.read_exact(&mut buf).unwrap();
        assert_eq!(buf, precious.as_bytes());
    }

    #[test]
    fn test_image_copy_default_first_mb() {
        let len: usize = 2 * 1024 * 1024;
        let mb: usize = 1024 * 1024;

        let mut data = vec![0u8; len];
        for i in 0..data.len() {
            data[i] = (i % 256) as u8;
        }

        // no saved partitions
        let mut source = Cursor::new(&data);
        let mut dest = tempfile::tempfile().unwrap();
        // copy
        source.seek(SeekFrom::Start(mb as u64)).unwrap();
        image_copy_default(&data[0..mb], &mut source, &mut dest, Path::new("/z"), None).unwrap();
        // compare
        dest.seek(SeekFrom::Start(0)).unwrap();
        let mut result = vec![0u8; len];
        dest.read_exact(&mut result).unwrap();
        assert_eq!(data, result);

        // SavedPartitions but nothing saved
        let mut source = Cursor::new(&data);
        let mut dest = tempfile::tempfile().unwrap();
        // gptman requires a fixed disk length
        dest.set_len(len as u64).unwrap();
        // create saved
        let saved = SavedPartitions::new_from_file(&mut dest, 512, &vec![]).unwrap();
        assert!(!saved.is_saved());
        // copy
        source.seek(SeekFrom::Start(mb as u64)).unwrap();
        image_copy_default(
            &data[0..mb],
            &mut source,
            &mut dest,
            Path::new("/z"),
            Some(&saved),
        )
        .unwrap();
        // compare
        dest.seek(SeekFrom::Start(0)).unwrap();
        let mut result = vec![0u8; len];
        dest.read_exact(&mut result).unwrap();
        assert_eq!(data, result);

        // saved partition
        let mut source = Cursor::new(data.clone());
        let mut dest = tempfile::tempfile().unwrap();
        // source must have a partition table
        partition(&mut source, None);
        let data_partitioned = source.into_inner();
        let mut source = Cursor::new(&data_partitioned);
        // gptman requires a fixed disk length
        dest.set_len(2 * len as u64).unwrap();
        // create partition to save
        partition(&mut dest, Some(2));
        // create saved
        let saved = SavedPartitions::new_from_file(
            &mut dest,
            512,
            &vec![PartitionFilter::Label(glob::Pattern::new("bovik").unwrap())],
        )
        .unwrap();
        assert!(saved.is_saved());
        // copy
        source.seek(SeekFrom::Start(mb as u64)).unwrap();
        image_copy_default(
            &data_partitioned[0..mb],
            &mut source,
            &mut dest,
            Path::new("/z"),
            Some(&saved),
        )
        .unwrap();
        // compare
        dest.seek(SeekFrom::Start(0)).unwrap();
        let mut result = vec![0u8; len];
        dest.read_exact(&mut result).unwrap();
        assert_eq!(detect_formatted_sector_size(&result), NonZeroU32::new(512));
        // boot code must match install data; partition table will not
        assert_eq!(data_partitioned[0..446], result[0..446]);
        let gpt_size = get_gpt_size(&mut dest).unwrap();
        assert!(gpt_size < 24576);
        assert_eq!(
            data_partitioned[gpt_size as usize..],
            result[gpt_size as usize..]
        );
    }

    fn partition(f: &mut (impl Read + Write + Seek), start_mb: Option<u64>) {
        let mut gpt = GPT::new_from(f, 512, *Uuid::new_v4().as_bytes()).unwrap();
        if let Some(start_mb) = start_mb {
            gpt[1] = GPTPartitionEntry {
                partition_type_guid: [1u8; 16],
                unique_partition_guid: [1u8; 16],
                starting_lba: start_mb * 2048,
                ending_lba: (start_mb + 1) * 2048,
                attribute_bits: 0,
                partition_name: "bovik".into(),
            };
        }
        gpt.write_into(f).unwrap();
    }
}
