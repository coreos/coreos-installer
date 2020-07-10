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

use byte_unit::Byte;
use error_chain::bail;
use flate2::read::GzDecoder;
use nix::unistd::isatty;
use progress_streams::ProgressReader;
use std::fs::{remove_file, File, OpenOptions};
use std::io::{stderr, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use xz2::read::XzDecoder;

use crate::blockdev::detect_formatted_sector_size;
use crate::cmdline::*;
use crate::errors::*;
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
        return Err("no signature available; can't check existing file".into());
    }
    let signature = source.signature.as_ref().unwrap();

    // compare signature to expectation
    let mut sig_file = OpenOptions::new()
        .read(true)
        .open(sig_path)
        .chain_err(|| format!("opening {}", sig_path.display()))?;
    let mut buf = Vec::new();
    sig_file
        .read_to_end(&mut buf)
        .chain_err(|| format!("reading {}", sig_path.display()))?;
    if &buf != signature {
        return Err("signature file doesn't match source".into());
    }

    // open image file
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .chain_err(|| format!("opening {}", path.display()))?;

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
        .chain_err(|| format!("opening {}", path.display()))?;

    // download and verify image
    // don't check sector size
    write_image(
        source,
        &mut dest,
        path,
        image_copy_default,
        decompress,
        None,
    )?;

    // write signature, if relevant
    if let (false, Some(signature)) = (decompress, source.signature.as_ref()) {
        let mut sig_dest = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(sig_path)
            .chain_err(|| format!("opening {}", sig_path.display()))?;
        sig_dest
            .write_all(signature)
            .chain_err(|| "writing signature data")?;
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
    expected_sector_size: Option<NonZeroU32>,
) -> Result<()>
where
    F: FnOnce(&[u8], &mut dyn Read, &mut File, &Path) -> Result<()>,
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
    let stderr_is_tty = isatty(stderr().as_raw_fd()).chain_err(|| "checking if stderr is a TTY")?;
    let (progress_prologue, progress_epilogue) = if stderr_is_tty {
        // Draw a status line that updates itself in place.
        // The prologue leaves a place for the cursor to rest between updates.
        // The epilogue writes three spaces to cover the switch from e.g.
        // 1000 KiB to 1 MiB, and then uses CR to return to the start of
        // the line.
        ("> ", "   \r")
    } else {
        // stderr is being read by another process, e.g. journald, and
        // fanciness may confuse it.  Just log regular lines.
        ("", "\n")
    };
    let artifact_type = source.artifact_type.clone();
    let have_length = source.length_hint.is_some();
    let length_hint = source.length_hint.unwrap_or(0);
    let mut position: u64 = 0;
    let mut last_report = Instant::now();
    let mut progress_reader = ProgressReader::new(&mut verify_reader, |progress: usize| {
        position += progress as u64;
        if last_report.elapsed() >= Duration::from_secs(1)
            || (have_length && position == length_hint)
        {
            last_report = Instant::now();
            if have_length {
                eprint!(
                    "{}Read {} {}/{} ({}%){}",
                    progress_prologue,
                    &artifact_type,
                    format_bytes(position),
                    format_bytes(length_hint),
                    100 * position / length_hint,
                    progress_epilogue
                );
            } else {
                eprint!(
                    "{}Read {} {}{}",
                    progress_prologue,
                    &artifact_type,
                    format_bytes(position),
                    progress_epilogue
                );
            }
            let _ = std::io::stdout().flush();
        }
    });

    // Wrap in a BufReader so we can peek at the first few bytes for
    // format sniffing.  Don't trust the content-type since the server
    // may not be configured correctly, or the file might be local.
    // Then wrap in a reader for decompression.
    let mut buf_reader = BufReader::new(&mut progress_reader);
    let mut decompress_reader: Box<dyn Read> = {
        let sniff = buf_reader.fill_buf().chain_err(|| "sniffing input")?;
        // verify default buffer size >= the largest magic number we might
        // care about
        assert!(sniff.len() >= 8);
        if !decompress {
            Box::new(buf_reader)
        } else if &sniff[0..2] == b"\x1f\x8b" {
            Box::new(GzDecoder::new(buf_reader))
        } else if &sniff[0..6] == b"\xfd7zXZ\x00" {
            Box::new(XzDecoder::new(buf_reader))
        } else {
            Box::new(buf_reader)
        }
    };

    // Read the first MiB of input and, if requested, check it against the
    // image's formatted sector size.
    let mut first_mb = [0u8; 1024 * 1024];
    decompress_reader
        .read_exact(&mut first_mb)
        .chain_err(|| "decoding first MiB of image")?;
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
    image_copy(&first_mb, &mut decompress_reader, dest, dest_path)?;

    // flush
    dest.flush().chain_err(|| "flushing data to disk")?;
    dest.sync_all().chain_err(|| "syncing data to disk")?;

    // if we reported progress using CRs, log final newline
    if stderr_is_tty {
        eprintln!();
    }

    Ok(())
}

pub fn image_copy_default(
    first_mb: &[u8],
    source: &mut dyn Read,
    dest: &mut File,
    _dest_path: &Path,
) -> Result<()> {
    // Cache the first MiB and write zeroes to dest instead.  This ensures
    // that the disk image can't be used accidentally before its GPG signature
    // is verified.
    dest.write_all(&[0u8; 1024 * 1024])
        .chain_err(|| "clearing first MiB of disk")?;

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
    copy(source, dest).chain_err(|| "decoding and writing image")?;

    // verify_reader has now checked the signature, so fill in the first MiB
    dest.seek(SeekFrom::Start(0))
        .chain_err(|| "seeking to start of disk")?;
    dest.write_all(first_mb)
        .chain_err(|| "writing to first MiB of disk")?;

    Ok(())
}

pub fn download_to_tempfile(url: &str) -> Result<File> {
    let mut f = tempfile::tempfile()?;

    let client = new_http_client()?;
    let mut resp = client
        .get(url)
        .send()
        .chain_err(|| format!("sending request for '{}'", url))?
        .error_for_status()
        .chain_err(|| format!("fetching '{}'", url))?;

    copy(&mut resp, &mut f)?;
    f.seek(SeekFrom::Start(0))
        .chain_err(|| format!("rewinding file for '{}'", url))?;

    Ok(f)
}

/// Format a size in bytes.
fn format_bytes(count: u64) -> String {
    Byte::from_bytes(count.into())
        .get_appropriate_unit(true)
        .format(1)
}
