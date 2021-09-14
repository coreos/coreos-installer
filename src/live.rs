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
use bytes::Buf;
use cpio::{write_cpio, NewcBuilder, NewcReader};
use nix::unistd::isatty;
use openat_ext::FileExt;
use serde::Serialize;
use std::convert::TryInto;
use std::fs::{read, write, File, OpenOptions};
use std::io::{self, copy, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::iter::repeat;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use crate::cmdline::*;
use crate::install::*;
use crate::io::*;
use crate::iso9660::{self, IsoFs};

const FILENAME: &str = "config.ign";
const COREOS_IGNITION_EMBED_PATH: &str = "IMAGES/IGNITION.IMG";
const COREOS_IGNITION_HEADER_SIZE: u64 = 24;
const COREOS_KARG_EMBED_AREA_HEADER_MAGIC: &[u8] = b"coreKarg";
const COREOS_KARG_EMBED_AREA_HEADER_SIZE: u64 = 72;
const COREOS_KARG_EMBED_AREA_HEADER_MAX_OFFSETS: usize = 6;
const COREOS_KARG_EMBED_AREA_MAX_SIZE: usize = 2048;
const COREOS_ISO_PXEBOOT_DIR: &str = "IMAGES/PXEBOOT";

pub fn iso_embed(config: &IsoEmbedConfig) -> Result<()> {
    eprintln!("`iso embed` is deprecated; use `iso ignition embed`.  Continuing.");
    iso_ignition_embed(&IsoIgnitionEmbedConfig {
        force: config.force,
        ignition_file: config.config.clone(),
        output: config.output.clone(),
        input: config.input.clone(),
    })
}

pub fn iso_show(config: &IsoShowConfig) -> Result<()> {
    eprintln!("`iso show` is deprecated; use `iso ignition show`.  Continuing.");
    iso_ignition_show(&IsoIgnitionShowConfig {
        input: config.input.clone(),
        header: false,
    })
}

pub fn iso_remove(config: &IsoRemoveConfig) -> Result<()> {
    eprintln!("`iso remove` is deprecated; use `iso ignition remove`.  Continuing.");
    iso_ignition_remove(&IsoIgnitionRemoveConfig {
        output: config.output.clone(),
        input: config.input.clone(),
    })
}

pub fn iso_ignition_embed(config: &IsoIgnitionEmbedConfig) -> Result<()> {
    let ignition = match config.ignition_file {
        Some(ref ignition_path) => {
            read(ignition_path).with_context(|| format!("reading {}", ignition_path))?
        }
        None => {
            let mut data = Vec::new();
            io::stdin()
                .lock()
                .read_to_end(&mut data)
                .context("reading stdin")?;
            data
        }
    };

    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    if !config.force && iso.have_ignition() {
        bail!("This ISO image already has an embedded Ignition config; use -f to force.");
    }

    let cpio = make_cpio(&ignition)?;
    iso.set_ignition(&cpio)?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_ignition_show(config: &IsoIgnitionShowConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if config.header {
        serde_json::to_writer_pretty(&mut out, &iso.ignition)
            .context("failed to serialize header")?;
        out.write_all(b"\n").context("failed to write newline")?;
    } else {
        if !iso.have_ignition() {
            bail!("No embedded Ignition config.");
        }
        out.write_all(&extract_cpio(iso.ignition())?)
            .context("writing output")?;
        out.flush().context("flushing output")?;
    }
    Ok(())
}

pub fn iso_ignition_remove(config: &IsoIgnitionRemoveConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    iso.set_ignition(&[])?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn pxe_ignition_wrap(config: &PxeIgnitionWrapConfig) -> Result<()> {
    if config.output.is_none()
        && isatty(io::stdout().as_raw_fd()).context("checking if stdout is a TTY")?
    {
        bail!("Refusing to write binary data to terminal");
    }

    let ignition = match config.ignition_file {
        Some(ref ignition_path) => {
            read(ignition_path).with_context(|| format!("reading {}", ignition_path))?
        }
        None => {
            let mut data = Vec::new();
            io::stdin()
                .lock()
                .read_to_end(&mut data)
                .context("reading stdin")?;
            data
        }
    };

    let cpio = make_cpio(&ignition)?;

    match &config.output {
        Some(output_path) => {
            write(output_path, cpio).with_context(|| format!("writing {}", output_path))?
        }
        None => {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            out.write_all(&cpio).context("writing output")?;
            out.flush().context("flushing output")?;
        }
    }
    Ok(())
}

pub fn pxe_ignition_unwrap(config: &PxeIgnitionUnwrapConfig) -> Result<()> {
    let buf = read(&config.input).with_context(|| format!("reading {}", config.input))?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(&extract_cpio(&buf)?)
        .context("writing output")?;
    out.flush().context("flushing output")?;
    Ok(())
}

pub fn iso_kargs_modify(config: &IsoKargsModifyConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    let kargs = modify_kargs(
        iso.kargs()?,
        &config.append,
        &[],
        &config.replace,
        &config.delete,
    )?;
    iso.set_kargs(&kargs)?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_reset(config: &IsoKargsResetConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    iso.set_kargs(&iso.kargs_default()?.to_string())?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_show(config: &IsoKargsShowConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    if config.header {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        serde_json::to_writer_pretty(&mut out, &iso.kargs).context("failed to serialize header")?;
        out.write_all(b"\n").context("failed to write newline")?;
    } else {
        let kargs = if config.default {
            iso.kargs_default()?
        } else {
            iso.kargs()?
        };
        println!("{}", kargs);
    }
    Ok(())
}

// output_path should be None if not outputting, or Some(output_path_argument)
fn open_live_iso(input_path: &str, output_path: Option<Option<&String>>) -> Result<File> {
    // if output_path is Some(None), we're modifying in place, so we need to
    // open for writing
    OpenOptions::new()
        .read(true)
        .write(matches!(output_path, Some(None)))
        .open(&input_path)
        .with_context(|| format!("opening {}", &input_path))
}

fn write_live_iso(iso: &IsoConfig, input: &mut File, output_path: Option<&String>) -> Result<()> {
    match output_path.map(|v| v.as_str()) {
        None => {
            // open_live_iso() opened input for writing
            iso.write(input)?;
        }
        Some("-") => {
            if isatty(io::stdout().as_raw_fd()).context("checking if stdout is a TTY")? {
                bail!("Refusing to write binary data to terminal");
            }
            iso.stream(input, &mut io::stdout().lock())?;
        }
        Some(output_path) => {
            let output_dir = Path::new(output_path)
                .parent()
                .with_context(|| format!("no parent directory of {}", output_path))?;
            let mut output = tempfile::Builder::new()
                .prefix(".coreos-installer-temp-")
                .tempfile_in(output_dir)
                .context("creating temporary file")?;
            input.seek(SeekFrom::Start(0)).context("seeking input")?;
            input
                .copy_to(output.as_file_mut())
                .context("copying input to temporary file")?;
            iso.write(output.as_file_mut())?;
            output
                .persist_noclobber(&output_path)
                .map_err(|e| e.error)
                .with_context(|| format!("persisting output file to {}", output_path))?;
        }
    }
    Ok(())
}

struct IsoConfig {
    ignition: Region,
    kargs: Option<KargEmbedAreas>,
}

impl IsoConfig {
    pub fn for_file(file: &mut File) -> Result<Self> {
        let mut iso = IsoFs::from_file(file.try_clone().context("cloning file")?)
            .context("parsing ISO9660 image")?;
        Ok(Self {
            ignition: ignition_embed_area(&mut iso)?,
            kargs: KargEmbedAreas::for_file(file)?,
        })
    }

    pub fn have_ignition(&self) -> bool {
        self.ignition().iter().any(|v| *v != 0)
    }

    pub fn ignition(&self) -> &[u8] {
        &self.ignition.contents[..]
    }

    pub fn set_ignition(&mut self, data: &[u8]) -> Result<()> {
        let capacity = self.ignition.length;
        if data.len() > capacity {
            bail!(
                "Compressed Ignition config is too large: {} > {}",
                data.len(),
                capacity
            )
        }
        self.ignition.contents.clear();
        self.ignition.contents.extend_from_slice(data);
        self.ignition
            .contents
            .extend(repeat(0).take(capacity - data.len()));
        self.ignition.modified = true;
        Ok(())
    }

    pub fn kargs(&self) -> Result<&str> {
        Ok(self.unwrap_kargs()?.kargs())
    }

    pub fn kargs_default(&self) -> Result<&str> {
        Ok(self.unwrap_kargs()?.kargs_default())
    }

    pub fn set_kargs(&mut self, kargs: &str) -> Result<()> {
        self.unwrap_kargs_mut()?.set_kargs(kargs)
    }

    fn unwrap_kargs(&self) -> Result<&KargEmbedAreas> {
        self.kargs
            .as_ref()
            .ok_or_else(|| anyhow!("No karg embed areas found; old or corrupted CoreOS ISO image."))
    }

    fn unwrap_kargs_mut(&mut self) -> Result<&mut KargEmbedAreas> {
        self.kargs
            .as_mut()
            .ok_or_else(|| anyhow!("No karg embed areas found; old or corrupted CoreOS ISO image."))
    }

    pub fn write(&self, file: &mut File) -> Result<()> {
        self.ignition.write(file)?;
        if let Some(kargs) = &self.kargs {
            kargs.write(file)?;
        }
        Ok(())
    }

    pub fn stream(&self, input: &mut File, writer: &mut (impl Write + ?Sized)) -> Result<()> {
        let mut regions = vec![&self.ignition];
        if let Some(kargs) = &self.kargs {
            regions.extend(kargs.regions.iter())
        }
        regions.stream(input, writer)
    }
}

#[derive(Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct Region {
    // sort order is derived from field order
    pub offset: u64,
    pub length: usize,
    #[serde(skip_serializing)]
    pub contents: Vec<u8>,
    #[serde(skip_serializing)]
    pub modified: bool,
}

impl Region {
    pub fn read(file: &mut File, offset: u64, length: usize) -> Result<Self> {
        let mut contents = vec![0; length];
        file.seek(SeekFrom::Start(offset))
            .with_context(|| format!("seeking to offset {}", offset))?;
        file.read_exact(&mut contents)
            .with_context(|| format!("reading {} bytes at {}", length, offset))?;
        Ok(Self {
            offset,
            length,
            contents,
            modified: false,
        })
    }

    pub fn write(&self, file: &mut File) -> Result<()> {
        self.validate()?;
        if self.modified {
            file.seek(SeekFrom::Start(self.offset))
                .with_context(|| format!("seeking to offset {}", self.offset))?;
            file.write_all(&self.contents)
                .with_context(|| format!("writing {} bytes at {}", self.length, self.offset))?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.length != self.contents.len() {
            bail!(
                "expected region contents length {}, found {}",
                self.length,
                self.contents.len()
            );
        }
        Ok(())
    }
}

trait Stream {
    fn stream(&self, input: &mut File, writer: &mut (impl Write + ?Sized)) -> Result<()>;
}

impl Stream for [&Region] {
    fn stream(&self, input: &mut File, writer: &mut (impl Write + ?Sized)) -> Result<()> {
        input.seek(SeekFrom::Start(0)).context("seeking to start")?;

        let mut regions: Vec<&&Region> = self.iter().filter(|r| r.modified).collect();
        regions.sort_unstable();

        let mut buf = [0u8; BUFFER_SIZE];
        let mut cursor: u64 = 0;

        // validate regions
        for region in &regions {
            region.validate()?;
            if region.offset < cursor {
                bail!(
                    "region starting at {} precedes current offset {}",
                    region.offset,
                    cursor
                );
            }
            cursor = region.offset + region.length as u64;
        }

        // write regions
        cursor = 0;
        for region in &regions {
            assert!(region.offset >= cursor);
            copy_exactly_n(input, writer, region.offset - cursor, &mut buf)
                .with_context(|| format!("copying bytes from {} to {}", cursor, region.offset))?;
            writer.write_all(&region.contents).with_context(|| {
                format!(
                    "writing region for {} at offset {}",
                    region.length, region.offset
                )
            })?;
            cursor = input
                .seek(SeekFrom::Current(region.length as i64))
                .with_context(|| format!("seeking region length {}", region.length))?;
        }

        // write the remainder
        let mut write_buf = BufWriter::with_capacity(BUFFER_SIZE, writer);
        copy(
            &mut BufReader::with_capacity(BUFFER_SIZE, input),
            &mut write_buf,
        )
        .context("copying file")?;
        write_buf.flush().context("flushing output")?;
        Ok(())
    }
}

#[derive(Serialize)]
struct KargEmbedAreas {
    length: usize,
    default: String,

    #[serde(rename = "kargs")]
    regions: Vec<Region>,
    #[serde(skip_serializing)]
    args: String,
}

impl KargEmbedAreas {
    // Return Ok(None) if no kargs embed areas exist.
    pub fn for_file(file: &mut File) -> Result<Option<Self>> {
        // The ISO 9660 System Area is 32 KiB. Karg embed area information is located in the 72 bytes
        // before the initrd embed area (see EmbedArea below):
        // 8 bytes: magic string "coreKarg"
        // 8 bytes little-endian: length of karg embed areas
        // 8 bytes little-endian: offset to default kargs
        // 8 bytes little-endian x 6: offsets to karg embed areas
        let region = Region::read(
            file,
            32768 - COREOS_IGNITION_HEADER_SIZE - COREOS_KARG_EMBED_AREA_HEADER_SIZE,
            COREOS_KARG_EMBED_AREA_HEADER_SIZE as usize,
        )
        .context("reading karg embed header")?;
        let mut header = &region.contents[..];
        // magic number
        if header.copy_to_bytes(8) != COREOS_KARG_EMBED_AREA_HEADER_MAGIC {
            return Ok(None);
        }
        // length
        let length: usize = header
            .get_u64_le()
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

        // we rely on Region::read() to verify that offset/length pairs are
        // in bounds

        // default kargs
        let offset = header.get_u64_le();
        let default_region = Region::read(file, offset, length).context("reading default kargs")?;
        let default = Self::parse(&default_region)?;

        // writable regions
        let mut regions = Vec::new();
        while regions.len() < COREOS_KARG_EMBED_AREA_HEADER_MAX_OFFSETS {
            let offset = header.get_u64_le();
            if offset == 0 {
                break;
            }
            regions.push(Region::read(file, offset, length).context("reading kargs embed area")?);
        }

        // we expect at least one
        if regions.is_empty() {
            bail!("No karg embed areas found; corrupted CoreOS ISO image.");
        }

        // parse kargs and verify that all the offsets have the same arguments
        let args = Self::parse(&regions[0])?;
        for region in regions.iter().skip(1) {
            let current_args = Self::parse(region)?;
            if current_args != args {
                bail!(
                    "kargs don't match at all offsets! (expected '{}', but offset {} has: '{}')",
                    args,
                    region.offset,
                    current_args
                );
            }
        }

        Ok(Some(KargEmbedAreas {
            length: default_region.length,
            default,
            regions,
            args,
        }))
    }

    fn parse(region: &Region) -> Result<String> {
        Ok(String::from_utf8(region.contents.clone())
            .context("invalid UTF-8 in karg area")?
            .trim_end_matches('#')
            .trim()
            .into())
    }

    pub fn kargs_default(&self) -> &str {
        &self.default
    }

    pub fn kargs(&self) -> &str {
        &self.args
    }

    pub fn set_kargs(&mut self, kargs: &str) -> Result<()> {
        let unformatted = kargs.trim();
        let formatted = unformatted.to_string() + "\n";
        if formatted.len() > self.length {
            bail!(
                "kargs too large for area: {} vs {}",
                formatted.len(),
                self.length
            );
        }
        let mut contents = vec![b'#'; self.length];
        contents[..formatted.len()].copy_from_slice(formatted.as_bytes());
        for region in &mut self.regions {
            region.contents = contents.clone();
            region.modified = true;
        }
        self.args = unformatted.to_string();
        Ok(())
    }

    pub fn write(&self, file: &mut File) -> Result<()> {
        for region in &self.regions {
            region.write(file)?;
        }
        Ok(())
    }
}

fn ignition_embed_area(iso: &mut IsoFs) -> Result<Region> {
    let f = iso
        .get_path(COREOS_IGNITION_EMBED_PATH)
        .context("finding Ignition embed area")?
        .try_into_file()?;
    // read (checks offset/length as a side effect)
    Region::read(iso.as_file()?, f.address.as_offset(), f.length as usize)
        .context("reading Ignition embed area")
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

#[derive(Serialize)]
struct IsoInspectOutput {
    header: IsoFs,
    records: Vec<String>,
}

pub fn iso_inspect(config: &IsoInspectConfig) -> Result<()> {
    let mut iso = IsoFs::from_file(open_live_iso(&config.input, None)?)?;
    let records = iso
        .walk()?
        .map(|r| r.map(|(path, _)| path))
        .collect::<Result<Vec<String>>>()
        .context("while walking ISO filesystem")?;
    let inspect_out = IsoInspectOutput {
        header: iso,
        records,
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, &inspect_out)
        .context("failed to serialize ISO metadata")?;
    out.write_all(b"\n").context("failed to write newline")?;
    Ok(())
}

pub fn iso_extract_pxe(config: &IsoExtractPxeConfig) -> Result<()> {
    let mut iso = IsoFs::from_file(open_live_iso(&config.input, None)?)?;
    let pxeboot = iso.get_path(COREOS_ISO_PXEBOOT_DIR)?.try_into_dir()?;
    std::fs::create_dir_all(&config.output_dir)?;

    let base = {
        // this can't be None since we successfully opened the live ISO at the location
        let mut s = Path::new(&config.input).file_stem().unwrap().to_os_string();
        s.push("-");
        s
    };

    for record in iso.list_dir(&pxeboot)? {
        match record? {
            iso9660::DirectoryRecord::Directory(_) => continue,
            iso9660::DirectoryRecord::File(file) => {
                let filename = {
                    let mut s = base.clone();
                    s.push(file.name.to_lowercase());
                    s
                };
                let path = Path::new(&config.output_dir).join(&filename);
                println!("{}", path.display());
                copy_file_from_iso(&mut iso, &file, &path)?;
            }
        }
    }
    Ok(())
}

fn copy_file_from_iso(iso: &mut IsoFs, file: &iso9660::File, output_path: &Path) -> Result<()> {
    let mut outf = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .with_context(|| format!("opening {}", output_path.display()))?;
    let mut bufw = BufWriter::with_capacity(BUFFER_SIZE, &mut outf);
    copy(&mut iso.read_file(file)?, &mut bufw)?;
    bufw.flush()?;
    Ok(())
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
