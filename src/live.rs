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

use cpio::{write_cpio, NewcBuilder, NewcReader};
use error_chain::bail;
use flate2::read::GzDecoder;
use flate2::{Compression, GzBuilder};
use std::convert::TryInto;
use std::fs::{remove_file, File, OpenOptions};
use std::io::{copy, stdin, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};

use crate::cmdline::*;
use crate::errors::*;
use crate::io::*;

const FILENAME: &str = "config.ign";

pub fn iso_embed(config: &IsoEmbedConfig) -> Result<()> {
    let mut holder = CopiedFileHolder::new(&config.input, config.output.as_ref())?;
    let mut embed = EmbedArea::for_file(&mut holder.file)?;

    let mut ignition: Vec<u8> = Vec::new();
    if let Some(ignition_path) = config.ignition.as_ref() {
        let mut file = OpenOptions::new()
            .read(true)
            .open(&ignition_path)
            .chain_err(|| format!("opening {}", ignition_path))?;
        file.read_to_end(&mut ignition)
            .chain_err(|| format!("reading {}", ignition_path))?;
    } else {
        stdin()
            .read_to_end(&mut ignition)
            .chain_err(|| "reading stdin")?;
    };

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
    // delete any existing config
    embed.clear()?;
    // write new config
    embed.seek_to_start()?;
    embed.write(&cpio)?;
    holder.complete();
    Ok(())
}

pub fn iso_show(config: &IsoShowConfig) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(&config.input)
        .chain_err(|| format!("opening {}", &config.input))?;
    let mut embed = EmbedArea::for_file(&mut file)?;

    embed.seek_to_start()?;
    let mut buf = embed.new_buffer();
    embed.read(&mut buf)?;
    // compare to zeroed buffer
    if buf == embed.new_buffer() {
        bail!("No embedded Ignition config.");
    }
    let config =
        String::from_utf8(extract_cpio(&buf)?).chain_err(|| "couldn't decode Ignition config")?;
    print!("{}", config);
    Ok(())
}

pub fn iso_remove(config: &IsoRemoveConfig) -> Result<()> {
    let mut holder = CopiedFileHolder::new(&config.input, config.output.as_ref())?;
    let mut embed = EmbedArea::for_file(&mut holder.file)?;
    embed.clear()?;
    holder.complete();
    Ok(())
}

struct CopiedFileHolder {
    pub file: File,
    copied_path: Option<String>,
    complete: bool,
}

/// Holder for a read/write file handle which is optionally copied from
/// another file.  If complete() is not called and the file was copied,
/// the copy will be deleted on drop.
impl CopiedFileHolder {
    fn new(input_path: &str, output_path: Option<&String>) -> Result<Self> {
        if let Some(unwrapped_output_path) = output_path {
            let mut input = OpenOptions::new()
                .read(true)
                .open(&input_path)
                .chain_err(|| format!("opening {}", &input_path))?;
            let mut output = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&unwrapped_output_path)
                .chain_err(|| format!("opening {}", &unwrapped_output_path))?;
            let mut writer = BufWriter::with_capacity(BUFFER_SIZE, &mut output);
            copy(
                &mut BufReader::with_capacity(BUFFER_SIZE, &mut input),
                &mut writer,
            )
            .chain_err(|| format!("copying {} to {}", input_path, unwrapped_output_path))?;
            writer
                .flush()
                .chain_err(|| format!("writing {}", unwrapped_output_path))?;
            drop(writer);
            Ok(CopiedFileHolder {
                file: output,
                copied_path: Some(unwrapped_output_path.to_string()),
                complete: false,
            })
        } else {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&input_path)
                .chain_err(|| format!("opening {}", &input_path))?;
            Ok(CopiedFileHolder {
                file,
                copied_path: None,
                complete: false,
            })
        }
    }

    fn complete(&mut self) {
        self.complete = true;
    }
}

impl Drop for CopiedFileHolder {
    fn drop(&mut self) {
        if self.copied_path.is_some() && !self.complete {
            let path = self.copied_path.as_ref().unwrap();
            if let Err(e) = remove_file(path) {
                eprintln!("Couldn't remove {}: {}", path, e);
            }
        }
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
        file.seek(SeekFrom::Start(32768 - 24))
            .chain_err(|| "seeking to embed header")?;
        // magic number
        file.read_exact(&mut buf)
            .chain_err(|| "reading embed header")?;
        if &buf != b"coreiso+" {
            bail!("Unrecognized CoreOS ISO image.");
        }
        // offset
        file.read_exact(&mut buf)
            .chain_err(|| "reading embed header")?;
        let offset = u64::from_le_bytes(buf);
        // length
        file.read_exact(&mut buf)
            .chain_err(|| "reading embed header")?;
        let length: usize = u64::from_le_bytes(buf)
            .try_into()
            .chain_err(|| "embed area too large to allocate")?;
        // check file size
        if file
            .seek(SeekFrom::End(0))
            .chain_err(|| "seeking to end of image file")?
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
            .chain_err(|| "seeking to embed area")?;
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<()> {
        self.file
            .read_exact(buf)
            .chain_err(|| "reading embed area")?;
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> Result<()> {
        self.file
            .write_all(buf)
            .chain_err(|| "writing embed area")?;
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
    let mut result = Cursor::new(Vec::new());
    let encoder = GzBuilder::new()
        .mtime(0)
        .write(&mut result, Compression::best());
    let mut input_files = vec![(
        // S_IFREG | 0644
        NewcBuilder::new(FILENAME).mode(0o100_644),
        Cursor::new(ignition),
    )];
    write_cpio(input_files.drain(..), encoder).chain_err(|| "writing CPIO archive")?;
    Ok(result.into_inner())
}

/// Extract a gzipped CPIO archive and return the contents of the Ignition
/// config.
fn extract_cpio(buf: &[u8]) -> Result<Vec<u8>> {
    let mut decompressor = GzDecoder::new(buf);
    loop {
        let mut reader = NewcReader::new(decompressor).chain_err(|| "reading CPIO entry")?;
        let entry = reader.entry();
        if entry.is_trailer() {
            bail!("couldn't find Ignition config in archive");
        }
        if entry.name() == FILENAME {
            let mut result = Vec::with_capacity(entry.file_size() as usize);
            reader
                .read_to_end(&mut result)
                .chain_err(|| "reading CPIO entry contents")?;
            return Ok(result);
        }
        decompressor = reader
            .finish()
            .chain_err(|| "finishing reading CPIO entry")?;
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
