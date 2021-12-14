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
use bytes::Buf;
use nix::unistd::isatty;
use openat_ext::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::{read, write, File, OpenOptions};
use std::io::{self, copy, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::iter::repeat;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::cmdline::*;
use crate::io::*;
use crate::iso9660::{self, IsoFs};
use crate::miniso;

const INITRD_IGNITION_PATH: &str = "config.ign";
const COREOS_IGNITION_EMBED_PATH: &str = "IMAGES/IGNITION.IMG";
const COREOS_IGNITION_HEADER_SIZE: u64 = 24;
const COREOS_KARG_EMBED_AREA_HEADER_MAGIC: &[u8] = b"coreKarg";
const COREOS_KARG_EMBED_AREA_HEADER_SIZE: u64 = 72;
const COREOS_KARG_EMBED_AREA_HEADER_MAX_OFFSETS: usize = 6;
const COREOS_KARG_EMBED_AREA_MAX_SIZE: usize = 2048;
const COREOS_KARG_EMBED_INFO_PATH: &str = "COREOS/KARGS.JSO";
const COREOS_ISO_PXEBOOT_DIR: &str = "IMAGES/PXEBOOT";
const COREOS_ISO_ROOTFS_IMG: &str = "IMAGES/PXEBOOT/ROOTFS.IMG";
const COREOS_ISO_MINISO_FILE: &str = "COREOS/MINISO.DAT";

pub fn iso_embed(config: IsoEmbedConfig) -> Result<()> {
    eprintln!("`iso embed` is deprecated; use `iso ignition embed`.  Continuing.");
    iso_ignition_embed(IsoIgnitionEmbedConfig {
        force: config.force,
        ignition_file: config.config,
        output: config.output,
        input: config.input,
    })
}

pub fn iso_show(config: IsoShowConfig) -> Result<()> {
    eprintln!("`iso show` is deprecated; use `iso ignition show`.  Continuing.");
    iso_ignition_show(IsoIgnitionShowConfig {
        input: config.input,
        header: false,
    })
}

pub fn iso_remove(config: IsoRemoveConfig) -> Result<()> {
    eprintln!("`iso remove` is deprecated; use `iso ignition remove`.  Continuing.");
    iso_ignition_remove(IsoIgnitionRemoveConfig {
        output: config.output,
        input: config.input,
    })
}

pub fn iso_ignition_embed(config: IsoIgnitionEmbedConfig) -> Result<()> {
    let ignition = match &config.ignition_file {
        Some(ignition_path) => {
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

    let cpio = make_initrd(&[(INITRD_IGNITION_PATH, &ignition)])?;
    iso.set_ignition(&cpio)?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_ignition_show(config: IsoIgnitionShowConfig) -> Result<()> {
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
        out.write_all(
            &extract_initrd(iso.ignition(), INITRD_IGNITION_PATH)?
                .context("couldn't find Ignition config in archive")?,
        )
        .context("writing output")?;
        out.flush().context("flushing output")?;
    }
    Ok(())
}

pub fn iso_ignition_remove(config: IsoIgnitionRemoveConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    iso.set_ignition(&[])?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn pxe_ignition_wrap(config: PxeIgnitionWrapConfig) -> Result<()> {
    if config.output.is_none() {
        verify_stdout_not_tty()?;
    }

    let ignition = match &config.ignition_file {
        Some(ignition_path) => {
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

    let cpio = make_initrd(&[(INITRD_IGNITION_PATH, &ignition)])?;

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

pub fn pxe_ignition_unwrap(config: PxeIgnitionUnwrapConfig) -> Result<()> {
    let stdin = io::stdin();
    let mut f: Box<dyn Read> = if let Some(path) = &config.input {
        Box::new(
            OpenOptions::new()
                .read(true)
                .open(path)
                .with_context(|| format!("opening {}", path))?,
        )
    } else {
        Box::new(stdin.lock())
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(
        &extract_initrd(&mut f, INITRD_IGNITION_PATH)?
            .context("couldn't find Ignition config in archive")?,
    )
    .context("writing output")?;
    out.flush().context("flushing output")?;
    Ok(())
}

pub fn iso_kargs_modify(config: IsoKargsModifyConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    let kargs = KargsEditor::new()
        .append(&config.append)
        .replace(&config.replace)
        .delete(&config.delete)
        .apply_to(iso.kargs()?)?;
    iso.set_kargs(&kargs)?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_reset(config: IsoKargsResetConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    iso.set_kargs(&iso.kargs_default()?.to_string())?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_show(config: IsoKargsShowConfig) -> Result<()> {
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
            verify_stdout_not_tty()?;
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
        IsoConfig::for_iso(&mut iso)
    }

    pub fn for_iso(iso: &mut IsoFs) -> Result<Self> {
        Ok(Self {
            ignition: ignition_embed_area(iso)?,
            kargs: KargEmbedAreas::for_iso(iso)?,
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
            .context("No karg embed areas found; old or corrupted CoreOS ISO image.")
    }

    fn unwrap_kargs_mut(&mut self) -> Result<&mut KargEmbedAreas> {
        self.kargs
            .as_mut()
            .context("No karg embed areas found; old or corrupted CoreOS ISO image.")
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

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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

#[derive(Deserialize, Serialize)]
struct KargEmbedInfo {
    default: String,
    files: Vec<KargEmbedLocation>,
    size: usize,
}

#[derive(Deserialize, Serialize)]
struct KargEmbedLocation {
    path: String,
    offset: u64,
}

impl KargEmbedInfo {
    // Returns Ok(None) if `kargs.json` doesn't exist.
    pub fn for_iso(iso: &mut IsoFs) -> Result<Option<Self>> {
        let iso_file = match iso.get_path(COREOS_KARG_EMBED_INFO_PATH) {
            Ok(record) => record.try_into_file()?,
            // old ISO without info JSON
            Err(e) if e.is::<iso9660::NotFound>() => return Ok(None),
            Err(e) => return Err(e),
        };
        let info: KargEmbedInfo = serde_json::from_reader(
            iso.read_file(&iso_file)
                .context("reading kargs embed area info")?,
        )
        .context("decoding kargs embed area info")?;
        Ok(Some(info))
    }

    pub fn update_iso(&self, iso: &mut IsoFs) -> Result<()> {
        let iso_file = iso.get_path(COREOS_KARG_EMBED_INFO_PATH)?.try_into_file()?;
        let mut w = iso.overwrite_file(&iso_file)?;
        let new_json = serde_json::to_string_pretty(&self).context("serializing object")?;
        if new_json.len() > iso_file.length as usize {
            // This really shouldn't happen. It's only used by the miniso stuff, and there we
            // strictly *remove* kargs from the default set.
            bail!(
                "New version of {} does not fit in space ({} vs {})",
                COREOS_KARG_EMBED_INFO_PATH,
                new_json.len(),
                iso_file.length,
            );
        }

        let mut contents = vec![b' '; iso_file.length as usize];
        contents[..new_json.len()].copy_from_slice(new_json.as_bytes());
        w.write_all(&contents)
            .with_context(|| format!("failed to update {}", COREOS_KARG_EMBED_INFO_PATH))?;
        w.flush().context("flushing ISO")?;
        Ok(())
    }
}

impl KargEmbedAreas {
    // Return Ok(None) if no kargs embed areas exist.
    pub fn for_iso(iso: &mut IsoFs) -> Result<Option<Self>> {
        let info = match KargEmbedInfo::for_iso(iso)? {
            Some(info) => info,
            None => return Self::for_file_via_system_area(iso.as_file()?),
        };

        // sanity-check size against a reasonable limit
        if info.size > COREOS_KARG_EMBED_AREA_MAX_SIZE {
            bail!(
                "karg embed area size larger than {} (found {})",
                COREOS_KARG_EMBED_AREA_MAX_SIZE,
                info.size
            );
        }
        if info.default.len() > info.size {
            bail!(
                "default kargs size {} larger than embed areas ({})",
                info.default.len(),
                info.size
            );
        }

        // writable regions
        let mut regions = Vec::new();
        for loc in info.files {
            let iso_file = iso
                .get_path(&loc.path.to_uppercase())
                .with_context(|| format!("looking up '{}'", loc.path))?
                .try_into_file()?;
            // we rely on Region::read() to verify that the offset/length
            // pair is in bounds
            regions.push(
                Region::read(
                    iso.as_file()?,
                    iso_file.address.as_offset() + loc.offset,
                    info.size,
                )
                .context("reading kargs embed area")?,
            );
        }
        regions.sort_unstable_by_key(|r| r.offset);

        Some(Self::build(info.size, info.default, regions)).transpose()
    }

    fn for_file_via_system_area(file: &mut File) -> Result<Option<Self>> {
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

        Some(Self::build(length, default, regions)).transpose()
    }

    fn build(length: usize, default: String, regions: Vec<Region>) -> Result<Self> {
        // we expect at least one region
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

        Ok(Self {
            length,
            default,
            regions,
            args,
        })
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

#[derive(Serialize)]
struct IsoInspectOutput {
    header: IsoFs,
    records: Vec<String>,
}

pub fn iso_inspect(config: IsoInspectConfig) -> Result<()> {
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

pub fn iso_extract_pxe(config: IsoExtractPxeConfig) -> Result<()> {
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
    bufw.flush().context("flushing buffer")?;
    Ok(())
}

pub fn iso_extract_minimal_iso(config: IsoExtractMinimalIsoConfig) -> Result<()> {
    // Note we don't support overwriting the input ISO. Unlike other commands, this operation is
    // non-reversible, so let's make it harder for users to shoot themselves in the foot.
    let mut full_iso = IsoFs::from_file(open_live_iso(&config.input, None)?)?;

    // For now, we require the full ISO to be completely vanilla. Otherwise, the hashes won't
    // match.
    let iso = IsoConfig::for_iso(&mut full_iso)?;
    if iso.have_ignition() {
        bail!("Cannot operate on ISO with embedded Ignition config. Reset it and try again.");
    } else if iso.kargs()? != iso.kargs_default()? {
        bail!("Cannot operate on ISO with non-default kargs. Reset it and try again.");
    }

    // do this early so we exit immediately if stdout is a TTY
    let output_dir: PathBuf = if &config.output == "-" {
        verify_stdout_not_tty()?;
        std::env::temp_dir()
    } else {
        Path::new(&config.output)
            .parent()
            .with_context(|| format!("no parent directory of {}", &config.output))?
            .into()
    };

    if let Some(path) = &config.output_rootfs {
        let rootfs = full_iso
            .get_path(COREOS_ISO_ROOTFS_IMG)
            .with_context(|| format!("looking up '{}'", COREOS_ISO_ROOTFS_IMG))?
            .try_into_file()?;
        copy_file_from_iso(&mut full_iso, &rootfs, Path::new(path))?;
    }

    let miniso_data_file = full_iso
        .get_path(COREOS_ISO_MINISO_FILE)
        .with_context(|| format!("looking up '{}'", COREOS_ISO_MINISO_FILE))?
        .try_into_file()?;

    let data = {
        let mut f = full_iso.read_file(&miniso_data_file)?;
        miniso::Data::deserialize(&mut f).context("reading miniso data file")?
    };
    let mut outf = tempfile::Builder::new()
        .prefix(".coreos-installer-temp-")
        .tempfile_in(&output_dir)
        .context("creating temporary file")?;
    data.unxzpack(full_iso.as_file()?, &mut outf)
        .context("unpacking miniso")?;
    outf.seek(SeekFrom::Start(0))
        .context("seeking back to start of miniso tempfile")?;

    modify_miniso_kargs(outf.as_file_mut(), config.rootfs_url.as_ref())
        .context("modifying miniso kernel args")?;

    if &config.output == "-" {
        copy(&mut outf, &mut io::stdout().lock()).context("writing output")?;
    } else {
        outf.persist_noclobber(&config.output)
            .map_err(|e| e.error)?;
    }

    Ok(())
}

pub fn iso_pack_minimal_iso(config: IsoExtractPackMinimalIsoConfig) -> Result<()> {
    let mut full_iso = IsoFs::from_file(open_live_iso(&config.full, Some(None))?)?;
    let mut minimal_iso = IsoFs::from_file(open_live_iso(&config.minimal, None)?)?;

    let full_files = collect_iso_files(&mut full_iso)
        .with_context(|| format!("collecting files from {}", &config.full))?;
    let minimal_files = collect_iso_files(&mut minimal_iso)
        .with_context(|| format!("collecting files from {}", &config.minimal))?;
    if full_files.is_empty() {
        bail!("No files found in {}", &config.full);
    } else if minimal_files.is_empty() {
        bail!("No files found in {}", &config.minimal);
    }

    eprintln!("Packing minimal ISO");
    let (data, matches, skipped, written, written_compressed) =
        miniso::Data::xzpack(minimal_iso.as_file()?, &full_files, &minimal_files)
            .context("packing miniso")?;
    eprintln!("Matched {} files of {}", matches, minimal_files.len());

    eprintln!("Total bytes skipped: {}", skipped);
    eprintln!("Total bytes written: {}", written);
    eprintln!("Total bytes written (compressed): {}", written_compressed);

    eprintln!("Verifying that packed image matches digest");
    data.unxzpack(full_iso.as_file()?, std::io::sink())
        .context("unpacking miniso for verification")?;

    let miniso_entry = full_iso
        .get_path(COREOS_ISO_MINISO_FILE)
        .with_context(|| format!("looking up '{}'", COREOS_ISO_MINISO_FILE))?
        .try_into_file()?;
    let mut w = full_iso.overwrite_file(&miniso_entry)?;
    data.serialize(&mut w).context("writing miniso data file")?;
    w.flush().context("flushing full ISO")?;

    if config.consume {
        std::fs::remove_file(&config.minimal)
            .with_context(|| format!("consuming {}", &config.minimal))?;
    }

    eprintln!("Packing successful!");
    Ok(())
}

fn collect_iso_files(iso: &mut IsoFs) -> Result<HashMap<String, iso9660::File>> {
    iso.walk()?
        .filter_map(|r| match r {
            Err(e) => Some(Err(e)),
            Ok((s, iso9660::DirectoryRecord::File(f))) => Some(Ok((s, f))),
            Ok(_) => None,
        })
        .collect::<Result<HashMap<String, iso9660::File>>>()
        .context("while walking ISO filesystem")
}

fn modify_miniso_kargs(f: &mut File, rootfs_url: Option<&String>) -> Result<()> {
    let mut iso = IsoFs::from_file(f.try_clone().context("cloning a file")?)?;
    let mut cfg = IsoConfig::for_file(f)?;

    let kargs = cfg.kargs()?;

    // same disclaimer as `modify_kargs()` here re. whitespace/quoting
    let liveiso_karg = kargs
        .split_ascii_whitespace()
        .find(|&karg| karg.starts_with("coreos.liveiso="))
        .context("minimal ISO does not have coreos.liveiso= karg")?
        .to_string();

    let new_default_kargs = KargsEditor::new().delete(&[liveiso_karg]).apply_to(kargs)?;
    cfg.set_kargs(&new_default_kargs)?;

    if let Some(url) = rootfs_url {
        if url.split_ascii_whitespace().count() > 1 {
            bail!("forbidden whitespace found in '{}'", url);
        }
        let final_kargs = KargsEditor::new()
            .append(&[format!("coreos.live.rootfs_url={}", url)])
            .apply_to(&new_default_kargs)?;

        cfg.set_kargs(&final_kargs)?;
    }

    // update kargs
    write_live_iso(&cfg, f, None)?;

    // also modify the default kargs because we don't want `coreos-installer iso kargs reset` to
    // re-add `coreos.liveiso`
    let mut kargs_info = KargEmbedInfo::for_iso(&mut iso)?.context(
        // should be impossible; we only support new-style CoreOS ISOs with kargs.json
        "minimal ISO does not have kargs.json; please report this as a bug",
    )?;

    // NB: We don't need to update the length for this; it's a fixed property of the kargs files.
    // (Though its original value did depend on the original default kargs at build time.)
    kargs_info.default = new_default_kargs;
    kargs_info.update_iso(&mut iso)?;

    Ok(())
}

fn verify_stdout_not_tty() -> Result<()> {
    if isatty(io::stdout().as_raw_fd()).context("checking if stdout is a TTY")? {
        bail!("Refusing to write binary data to terminal");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::copy;

    use tempfile::tempfile;
    use xz2::read::XzDecoder;

    fn open_iso_file() -> File {
        let iso_bytes: &[u8] = include_bytes!("../fixtures/iso/embed-areas-2021-09.iso.xz");
        let mut decoder = XzDecoder::new(iso_bytes);
        let mut iso_file = tempfile().unwrap();
        copy(&mut decoder, &mut iso_file).unwrap();
        iso_file
    }

    #[test]
    fn test_ignition_embed_area() {
        let mut iso_file = open_iso_file();
        // normal read
        let mut iso = IsoFs::from_file(iso_file.try_clone().unwrap()).unwrap();
        let region = ignition_embed_area(&mut iso).unwrap();
        assert_eq!(region.offset, 102400);
        assert_eq!(region.length, 262144);
        // missing embed area
        iso_file.seek(SeekFrom::Start(65903)).unwrap();
        iso_file.write_all(b"Z").unwrap();
        let mut iso = IsoFs::from_file(iso_file).unwrap();
        ignition_embed_area(&mut iso).unwrap_err();
    }

    #[test]
    fn test_karg_embed_area() {
        let mut iso_file = open_iso_file();
        // normal read
        check_karg_embed_areas(&mut iso_file);
        // JSON only
        iso_file.seek(SeekFrom::Start(32672)).unwrap();
        iso_file.write_all(&[0; 8]).unwrap();
        check_karg_embed_areas(&mut iso_file);
        // legacy header only
        iso_file.seek(SeekFrom::Start(32672)).unwrap();
        iso_file.write_all(b"coreKarg").unwrap();
        iso_file.seek(SeekFrom::Start(63725)).unwrap();
        iso_file.write_all(b"Z").unwrap();
        check_karg_embed_areas(&mut iso_file);
        // neither header
        iso_file.seek(SeekFrom::Start(32672)).unwrap();
        iso_file.write_all(&[0; 8]).unwrap();
        let mut iso = IsoFs::from_file(iso_file).unwrap();
        assert!(KargEmbedAreas::for_iso(&mut iso).unwrap().is_none());
    }

    fn check_karg_embed_areas(iso_file: &mut File) {
        let iso_file = iso_file.try_clone().unwrap();
        let mut iso = IsoFs::from_file(iso_file).unwrap();
        let areas = KargEmbedAreas::for_iso(&mut iso).unwrap().unwrap();
        assert_eq!(areas.length, 1139);
        assert_eq!(areas.default, "mitigations=auto,nosmt coreos.liveiso=fedora-coreos-34.20210921.dev.0 ignition.firstboot ignition.platform.id=metal");
        assert_eq!(areas.regions.len(), 2);
        assert_eq!(areas.regions[0].offset, 98126);
        assert_eq!(areas.regions[0].length, 1139);
        assert_eq!(areas.regions[1].offset, 371658);
        assert_eq!(areas.regions[1].length, 1139);
    }
}
