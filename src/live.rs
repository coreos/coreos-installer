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

use anyhow::{bail, Context, Result};
use cpio::{write_cpio, NewcBuilder, NewcReader};
use nix::unistd::isatty;
use openat_ext::FileExt;
use serde::Serialize;
use std::convert::TryInto;
use std::fs::{read, write, File, OpenOptions};
use std::io::{copy, stdin, stdout, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::cmdline::*;
use crate::install::*;
use crate::io::*;

const FILENAME: &str = "config.ign";
const COREOS_IGNITION_HEADER_MAGIC: &[u8] = b"coreiso+";
const COREOS_IGNITION_HEADER_SIZE: u64 = 24;
const COREOS_KARG_EMBED_AREA_HEADER_MAGIC: &[u8] = b"coreKarg";
const COREOS_KARG_EMBED_AREA_HEADER_SIZE: u64 = 72;
const COREOS_KARG_EMBED_AREA_HEADER_MAX_OFFSETS: usize = 6;
const COREOS_KARG_EMBED_AREA_MAX_SIZE: usize = 2048;

pub fn iso_embed(config: &IsoIgnitionEmbedConfig) -> Result<()> {
    eprintln!("`iso embed` is deprecated; use `iso ignition embed`.  Continuing.");
    iso_ignition_embed(config)
}

pub fn iso_show(config: &IsoIgnitionShowConfig) -> Result<()> {
    eprintln!("`iso show` is deprecated; use `iso ignition show`.  Continuing.");
    iso_ignition_show(config)
}

pub fn iso_remove(config: &IsoIgnitionRemoveConfig) -> Result<()> {
    eprintln!("`iso remove` is deprecated; use `iso ignition remove`.  Continuing.");
    iso_ignition_remove(config)
}

pub fn iso_ignition_embed(config: &IsoIgnitionEmbedConfig) -> Result<()> {
    let ignition = match config.ignition {
        Some(ref ignition_path) => {
            read(ignition_path).with_context(|| format!("reading {}", ignition_path))?
        }
        None => {
            let mut data = Vec::new();
            stdin().read_to_end(&mut data).context("reading stdin")?;
            data
        }
    };

    let mut content = ContentFile::new(&config.input, config.output.as_ref())?;
    let use_stdout = content.is_stdout();
    let mut embed = EmbedArea::for_file(content.as_file_mut())?;

    let cpio = make_cpio(&ignition)?;
    if cpio.len() > embed.length {
        bail!(
            "Compressed Ignition config is too large: {} > {}",
            cpio.len(),
            embed.length
        );
    }
    if !config.force {
        // Ensure all zero bytes
        embed.seek_to_start()?;
        let mut buf = embed.new_buffer();
        embed.read(&mut buf)?;
        // compare to zeroed buffer
        if buf != embed.new_buffer() {
            bail!("This ISO image already has an embedded Ignition config; use -f to force.");
        }
    }

    if use_stdout {
        embed.stream(&cpio, &mut stdout())?;
    } else {
        // delete any existing config
        embed.clear()?;
        // write new config
        embed.seek_to_start()?;
        embed.write(&cpio)?;
    }
    content.complete()?;

    Ok(())
}

pub fn iso_ignition_show(config: &IsoIgnitionShowConfig) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(&config.input)
        .with_context(|| format!("opening {}", &config.input))?;
    let mut embed = EmbedArea::for_file(&mut file)?;

    embed.seek_to_start()?;
    let mut buf = embed.new_buffer();
    embed.read(&mut buf)?;
    // compare to zeroed buffer
    if buf == embed.new_buffer() {
        bail!("No embedded Ignition config.");
    }
    stdout()
        .write_all(&extract_cpio(&buf)?)
        .context("writing output")?;
    stdout().flush().context("flushing output")?;
    Ok(())
}

pub fn iso_ignition_remove(config: &IsoIgnitionRemoveConfig) -> Result<()> {
    let mut content = ContentFile::new(&config.input, config.output.as_ref())?;
    let use_stdout = content.is_stdout();
    let mut embed = EmbedArea::for_file(content.as_file_mut())?;

    if use_stdout {
        embed.stream(&[], &mut stdout())?;
    } else {
        embed.clear()?;
    }
    content.complete()?;
    Ok(())
}

pub fn pxe_ignition_wrap(config: &PxeIgnitionWrapConfig) -> Result<()> {
    if config.output.is_none()
        && isatty(stdout().as_raw_fd()).context("checking if stdout is a TTY")?
    {
        bail!("Refusing to write binary data to terminal");
    }

    let ignition = match config.ignition {
        Some(ref ignition_path) => {
            read(ignition_path).with_context(|| format!("reading {}", ignition_path))?
        }
        None => {
            let mut data = Vec::new();
            stdin().read_to_end(&mut data).context("reading stdin")?;
            data
        }
    };

    let cpio = make_cpio(&ignition)?;

    match &config.output {
        Some(output_path) => {
            write(output_path, cpio).with_context(|| format!("writing {}", output_path))?
        }
        None => {
            stdout().write_all(&cpio).context("writing output")?;
            stdout().flush().context("flushing output")?;
        }
    }
    Ok(())
}

pub fn pxe_ignition_unwrap(config: &PxeIgnitionUnwrapConfig) -> Result<()> {
    let buf = read(&config.input).with_context(|| format!("reading {}", config.input))?;
    stdout()
        .write_all(&extract_cpio(&buf)?)
        .context("writing output")?;
    stdout().flush().context("flushing output")?;
    Ok(())
}

pub fn iso_kargs_modify(config: &IsoKargsModifyConfig) -> Result<()> {
    let mut content = ContentFile::new(&config.input, config.output.as_ref())?;
    let use_stdout = content.is_stdout();
    let mut embed = KargEmbedAreas::for_file(content.as_file_mut())?;

    let current_kargs = embed.get_current_kargs()?;
    let new_kargs = modify_kargs(
        &current_kargs,
        &config.append,
        &[],
        &config.replace,
        &config.delete,
    )?;
    if use_stdout {
        embed.stream(&new_kargs, &mut stdout())?;
    } else {
        embed.write_kargs(&new_kargs)?;
    }
    content.complete()?;
    Ok(())
}

pub fn iso_kargs_reset(config: &IsoKargsResetConfig) -> Result<()> {
    let mut content = ContentFile::new(&config.input, config.output.as_ref())?;
    let use_stdout = content.is_stdout();
    let mut embed = KargEmbedAreas::for_file(content.as_file_mut())?;

    let default_kargs = embed.get_default_kargs()?;
    if use_stdout {
        embed.stream(&default_kargs, &mut stdout())?;
    } else {
        embed.write_kargs(&default_kargs)?;
    }
    content.complete()?;
    Ok(())
}

pub fn iso_kargs_show(config: &IsoKargsShowConfig) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(&config.input)
        .with_context(|| format!("opening {}", &config.input))?;
    let mut embed = KargEmbedAreas::for_file(&mut file)?;
    if config.header {
        serde_json::to_writer_pretty(std::io::stdout(), &embed)
            .context("failed to serialize header")?;
    } else {
        let kargs = if config.default {
            embed.get_default_kargs()?
        } else {
            embed.get_current_kargs()?
        };
        println!("{}", kargs);
    }
    Ok(())
}

#[derive(Serialize)]
struct KargEmbedAreas<'a> {
    #[serde(skip_serializing)]
    file: &'a mut File,
    length: usize,
    default_kargs_offset: u64,
    kargs_offsets: Vec<u64>,
}

impl<'a> KargEmbedAreas<'a> {
    fn for_file(file: &'a mut File) -> Result<Self> {
        let mut buf: [u8; 8] = [0; 8];
        // The ISO 9660 System Area is 32 KiB. Karg embed area information is located in the 72 bytes
        // before the initrd embed area (see EmbedArea below):
        // 8 bytes: magic string "coreKarg"
        // 8 bytes little-endian: length of karg embed areas
        // 8 bytes little-endian: offset to default kargs
        // 8 bytes little-endian x 6: offsets to karg embed areas
        file.seek(SeekFrom::Start(
            32768 - COREOS_IGNITION_HEADER_SIZE - COREOS_KARG_EMBED_AREA_HEADER_SIZE,
        ))
        .context("seeking to karg embed header")?;
        // magic number
        file.read_exact(&mut buf)
            .context("reading karg embed header")?;
        if buf != COREOS_KARG_EMBED_AREA_HEADER_MAGIC {
            bail!("No karg embed areas found; old or corrupted CoreOS ISO image.");
        }
        // length
        file.read_exact(&mut buf)
            .context("reading karg embed header")?;
        let length: usize = u64::from_le_bytes(buf)
            .try_into()
            .context("karg embed area length too large to allocate")?;
        // sanity-check against a reasonable limit
        if length > COREOS_KARG_EMBED_AREA_MAX_SIZE {
            bail!(
                "karg embed area length larger than {} (found {})",
                COREOS_KARG_EMBED_AREA_MAX_SIZE,
                length
            );
        }

        let metadata = file.metadata().context("reading metadata for ISO")?;
        let iso_size = metadata.len();

        // default kargs
        file.read_exact(&mut buf)
            .context("reading karg embed header")?;
        let default_kargs_offset: u64 = u64::from_le_bytes(buf);
        if default_kargs_offset + (length as u64) > iso_size {
            bail!(
                "Default kargs area end outside ISO ({}+{} vs {})",
                default_kargs_offset,
                length,
                iso_size
            );
        }

        // offsets
        let mut kargs_offsets: Vec<u64> = Vec::new();
        while kargs_offsets.len() < COREOS_KARG_EMBED_AREA_HEADER_MAX_OFFSETS {
            file.read_exact(&mut buf)
                .context("reading karg embed header")?;
            let offset: u64 = u64::from_le_bytes(buf);
            if offset == 0 {
                break;
            } else if offset + (length as u64) > iso_size {
                bail!(
                    "Kargs area end outside ISO ({}+{} vs {})",
                    offset,
                    length,
                    iso_size
                );
            }
            kargs_offsets.push(offset);
        }

        // we need ordered offsets when streaming
        kargs_offsets.sort_unstable();

        // we expect at least one
        if kargs_offsets.is_empty() {
            bail!("No karg embed areas found; corrupted CoreOS ISO image.");
        }

        Ok(KargEmbedAreas {
            file,
            length,
            default_kargs_offset,
            kargs_offsets,
        })
    }

    fn get_current_kargs(&mut self) -> Result<String> {
        // really, we could just get the kargs from the first file, but let's sanity-check that all
        // the offsets have the same kargs
        let mut first_kargs: Option<String> = None;
        for offset in &self.kargs_offsets {
            let kargs = get_kargs_at_offset(self.file, self.length, *offset)?;
            if let Some(ref first_kargs) = first_kargs {
                if &kargs != first_kargs {
                    bail!(
                        "kargs don't match at all offsets! (expected '{}', but offset {} has: '{}')",
                        first_kargs,
                        *offset,
                        kargs
                    );
                }
            } else {
                first_kargs = Some(kargs);
            }
        }
        Ok(first_kargs.expect("at least one area offset"))
    }

    fn get_default_kargs(&mut self) -> Result<String> {
        get_kargs_at_offset(self.file, self.length, self.default_kargs_offset)
    }

    fn format_embed_area(&mut self, kargs: &str) -> Result<Vec<u8>> {
        let kargs: String = kargs.trim().to_string() + "\n";
        if kargs.len() > self.length {
            bail!(
                "kargs too large for area: {} vs {}",
                kargs.len(),
                self.length
            );
        }
        let mut new_area = vec![b'#'; self.length];
        new_area[..kargs.len()].copy_from_slice(kargs.as_bytes());
        Ok(new_area)
    }

    fn stream(&mut self, kargs: &str, writer: &mut (impl Write + ?Sized)) -> Result<()> {
        let mut buf = [0u8; BUFFER_SIZE];
        let new_area = self.format_embed_area(&kargs)?;

        self.file
            .seek(SeekFrom::Start(0))
            .context("seeking to start")?;
        let mut cursor: u64 = 0;

        for offset in &self.kargs_offsets {
            copy_exactly_n(&mut self.file, writer, *offset - cursor, &mut buf)
                .with_context(|| format!("copying bytes from {} to {}", cursor, *offset))?;
            writer
                .write_all(&new_area)
                .with_context(|| format!("writing karg embed area at offset {}", *offset))?;
            cursor = self
                .file
                .seek(SeekFrom::Current(self.length as i64))
                .with_context(|| format!("seeking length of karg embed area {}", self.length))?;
        }

        // write the remainder
        let mut write_buf = BufWriter::with_capacity(BUFFER_SIZE, writer);
        copy(
            &mut BufReader::with_capacity(BUFFER_SIZE, &mut self.file),
            &mut write_buf,
        )
        .context("copying file")?;
        write_buf.flush().context("flushing output")?;
        Ok(())
    }

    fn write_kargs(&mut self, kargs: &str) -> Result<()> {
        let new_area = self.format_embed_area(&kargs)?;

        for offset in &self.kargs_offsets {
            self.file
                .seek(SeekFrom::Start(*offset))
                .with_context(|| format!("seeking to karg area offset {}", *offset))?;
            self.file
                .write_all(&new_area)
                .with_context(|| format!("writing karg embed area at offset {}", *offset))?;
        }
        Ok(())
    }
}

// This is purposely not an impl function because we need to be able to call it while borrowing
// parts of the struct (e.g. when iterating over the offsets).
fn get_kargs_at_offset(file: &mut File, area_length: usize, offset: u64) -> Result<String> {
    file.seek(SeekFrom::Start(offset))
        .with_context(|| format!("seeking to karg area offset {}", offset))?;
    let area = {
        let mut buf = vec![0u8; area_length];
        file.read_exact(&mut buf)
            .with_context(|| format!("reading karg area at offset {}", offset))?;
        String::from_utf8(buf).context("invalid UTF-8 in karg area")?
    };
    Ok(area.trim_end_matches('#').trim().into())
}

enum ContentFile {
    ForStdout(File),
    InPlace(File),
    Copied((NamedTempFile, PathBuf)),
}

/// Wrapper for a file handle to the content being modified (for example, an
/// ISO image).  Usually this is where we write our modifications, but if
/// we're streaming to stdout, it's where we read the content from.  In the
/// case of an output file, it can be modified in place or copied from
/// another file.  If complete() is not called and the file was copied, the
/// copy will be deleted on drop.
impl ContentFile {
    fn new(input_path: &str, output_path: Option<&String>) -> Result<Self> {
        match output_path.map(|v| v.as_str()) {
            None => Ok(Self::InPlace(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&input_path)
                    .with_context(|| format!("opening {}", &input_path))?,
            )),
            Some("-") => {
                // check this here as a convenience to the caller
                if isatty(stdout().as_raw_fd()).context("checking if stdout is a TTY")? {
                    bail!("Refusing to write binary data to terminal");
                }
                // read-only for safety
                Ok(Self::ForStdout(
                    OpenOptions::new()
                        .read(true)
                        .open(&input_path)
                        .with_context(|| format!("opening {}", &input_path))?,
                ))
            }
            Some(unwrapped_output_path) => {
                let output_dir = Path::new(unwrapped_output_path)
                    .parent()
                    .with_context(|| format!("no parent directory of {}", unwrapped_output_path))?;
                let input = OpenOptions::new()
                    .read(true)
                    .open(&input_path)
                    .with_context(|| format!("opening {}", &input_path))?;
                let mut output = tempfile::Builder::new()
                    .prefix(".coreos-installer-temp-")
                    .tempfile_in(output_dir)
                    .context("creating temporary file")?;
                input
                    .copy_to(output.as_file_mut())
                    .with_context(|| format!("copying {} to temporary file", input_path))?;
                Ok(Self::Copied((
                    output,
                    Path::new(unwrapped_output_path).to_path_buf(),
                )))
            }
        }
    }

    fn is_stdout(&self) -> bool {
        matches!(self, Self::ForStdout(_))
    }

    // Return the output file for InPlace and Copied, and the input file
    // for ForStdout.
    fn as_file_mut(&mut self) -> &mut File {
        match self {
            Self::ForStdout(ref mut file) => file,
            Self::InPlace(ref mut file) => file,
            Self::Copied((temp, _)) => temp.as_file_mut(),
        }
    }

    fn complete(self) -> Result<()> {
        match self {
            Self::ForStdout(_) => (),
            Self::InPlace(_) => (),
            Self::Copied((temp, path)) => {
                temp.persist_noclobber(&path)
                    .map_err(|e| e.error)
                    .with_context(|| format!("persisting output file to {}", path.display()))?;
            }
        }
        Ok(())
    }
}

struct EmbedArea<'a> {
    file: &'a File,
    offset: u64,
    length: usize,
}

impl<'a> EmbedArea<'a> {
    fn for_file(file: &'a mut File) -> Result<Self> {
        let mut buf: [u8; 8] = [0; 8];
        // The ISO 9660 System Area is 32 KiB.  The last 24 bytes should be:
        // 8 bytes: magic string "coreiso+"
        // 8 bytes little-endian: offset of embed area from start of file
        // 8 bytes little-endian: length of embed area
        file.seek(SeekFrom::Start(32768 - COREOS_IGNITION_HEADER_SIZE))
            .context("seeking to embed header")?;
        // magic number
        file.read_exact(&mut buf).context("reading embed header")?;
        if buf != COREOS_IGNITION_HEADER_MAGIC {
            bail!("Unrecognized CoreOS ISO image.");
        }
        // offset
        file.read_exact(&mut buf).context("reading embed header")?;
        let offset = u64::from_le_bytes(buf);
        // length
        file.read_exact(&mut buf).context("reading embed header")?;
        let length: usize = u64::from_le_bytes(buf)
            .try_into()
            .context("embed area too large to allocate")?;
        // check file size
        if file
            .seek(SeekFrom::End(0))
            .context("seeking to end of image file")?
            < offset + length as u64
        {
            bail!("Invalid CoreOS ISO image.");
        }
        Ok(Self {
            file,
            offset,
            length,
        })
    }

    fn seek_to_start(&mut self) -> Result<()> {
        self.file
            .seek(SeekFrom::Start(self.offset))
            .context("seeking to embed area")?;
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<()> {
        self.file.read_exact(buf).context("reading embed area")?;
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> Result<()> {
        self.file.write_all(buf).context("writing embed area")?;
        Ok(())
    }

    fn stream(&mut self, cpio: &[u8], writer: &mut (impl Write + ?Sized)) -> Result<()> {
        let mut buf = [0u8; BUFFER_SIZE];
        self.file
            .seek(SeekFrom::Start(0))
            .context("seeking to start")?;
        copy_exactly_n(&mut self.file, writer, self.offset, &mut buf).context("copying file")?;
        if cpio.len() > self.length {
            bail!("buffer larger than embed area");
        }
        writer.write_all(cpio).context("writing embed area")?;
        let zeroes = vec![0; self.length - cpio.len()];
        writer.write_all(&zeroes).context("writing zeros")?;
        self.file
            .seek(SeekFrom::Start(self.offset + self.length as u64))
            .context("seeking to end of embed area")?;
        let mut write_buf = BufWriter::with_capacity(BUFFER_SIZE, writer);
        copy(
            &mut BufReader::with_capacity(BUFFER_SIZE, self.file),
            &mut write_buf,
        )
        .context("copying file")?;
        write_buf.flush().context("flushing output")?;
        Ok(())
    }

    /// Wipe the embed area.
    fn clear(&mut self) -> Result<()> {
        self.seek_to_start()?;
        let zeroes = self.new_buffer();
        self.write(&zeroes)?;
        Ok(())
    }

    /// Allocate a zeroed buffer the size of the embed area.
    fn new_buffer(&self) -> Vec<u8> {
        vec![0; self.length]
    }
}

/// Make a gzipped CPIO archive containing the specified Ignition config.
fn make_cpio(ignition: &[u8]) -> Result<Vec<u8>> {
    use xz2::stream::{Check, Stream};
    use xz2::write::XzEncoder;

    let mut result = Cursor::new(Vec::new());
    // kernel requires CRC32: https://www.kernel.org/doc/Documentation/xz.txt
    let encoder = XzEncoder::new_stream(
        &mut result,
        Stream::new_easy_encoder(9, Check::Crc32).context("creating XZ encoder")?,
    );
    let mut input_files = vec![(
        // S_IFREG | 0644
        NewcBuilder::new(FILENAME).mode(0o100_644),
        Cursor::new(ignition),
    )];
    write_cpio(input_files.drain(..), encoder).context("writing CPIO archive")?;
    Ok(result.into_inner())
}

/// Extract a gzipped CPIO archive and return the contents of the Ignition
/// config.
fn extract_cpio(buf: &[u8]) -> Result<Vec<u8>> {
    // older versions of this program, and its predecessor, compressed
    // with gzip
    let mut decompressor = DecompressReader::new(BufReader::new(buf))?;
    loop {
        let mut reader = NewcReader::new(decompressor).context("reading CPIO entry")?;
        let entry = reader.entry();
        if entry.is_trailer() {
            bail!("couldn't find Ignition config in archive");
        }
        if entry.name() == FILENAME {
            let mut result = Vec::with_capacity(entry.file_size() as usize);
            reader
                .read_to_end(&mut result)
                .context("reading CPIO entry contents")?;
            return Ok(result);
        }
        decompressor = reader.finish().context("finishing reading CPIO entry")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpio_roundtrip() {
        let input = r#"{}"#;
        let cpio = make_cpio(input.as_bytes()).unwrap();
        let output = extract_cpio(&cpio).unwrap();
        assert_eq!(input.as_bytes(), output.as_slice());
    }
}
