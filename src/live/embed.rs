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

//! ISO embed area support

use anyhow::{bail, Context, Result};
use bytes::Buf;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{copy, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::iter::repeat;

use crate::io::*;
use crate::iso9660::{self, IsoFs};

pub(super) const INITRD_IGNITION_PATH: &str = "config.ign";
pub(super) const INITRD_NETWORK_DIR: &str = "etc/coreos-firstboot-network";

lazy_static! {
    pub(super) static ref INITRD_IGNITION_GLOB: GlobMatcher =
        GlobMatcher::new(&[INITRD_IGNITION_PATH]).unwrap();
    pub(super) static ref INITRD_NETWORK_GLOB: GlobMatcher =
        GlobMatcher::new(&[&format!("{}/*", INITRD_NETWORK_DIR)]).unwrap();
}

const COREOS_INITRD_EMBED_PATH: &str = "IMAGES/IGNITION.IMG";
const COREOS_INITRD_HEADER_SIZE: u64 = 24;
const COREOS_KARG_EMBED_AREA_HEADER_MAGIC: &[u8] = b"coreKarg";
const COREOS_KARG_EMBED_AREA_HEADER_SIZE: u64 = 72;
const COREOS_KARG_EMBED_AREA_HEADER_MAX_OFFSETS: usize = 6;
const COREOS_KARG_EMBED_AREA_MAX_SIZE: usize = 2048;
const COREOS_KARG_EMBED_INFO_PATH: &str = "COREOS/KARGS.JSO";

pub(super) struct IsoConfig {
    initrd: InitrdEmbedArea,
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
            initrd: InitrdEmbedArea::for_iso(iso).context("Unrecognized CoreOS ISO image.")?,
            kargs: KargEmbedAreas::for_iso(iso)?,
        })
    }

    pub fn have_ignition(&self) -> bool {
        self.initrd().get(INITRD_IGNITION_PATH).is_some()
    }

    pub fn have_network(&self) -> bool {
        !self.initrd().find(&INITRD_NETWORK_GLOB).is_empty()
    }

    pub fn remove_network(&mut self) {
        let initrd = self.initrd_mut();
        let paths: Vec<String> = initrd
            .find(&INITRD_NETWORK_GLOB)
            .keys()
            .map(|p| p.to_string())
            .collect();
        for path in paths {
            initrd.remove(&path);
        }
    }

    pub fn initrd(&self) -> &Initrd {
        self.initrd.initrd()
    }

    pub fn initrd_mut(&mut self) -> &mut Initrd {
        self.initrd.initrd_mut()
    }

    // for debugging
    pub fn initrd_header_json(&self) -> Result<Vec<u8>> {
        let mut ret =
            serde_json::to_vec_pretty(&self.initrd).context("failed to serialize initrd header")?;
        ret.push(b'\n');
        Ok(ret)
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

    pub fn kargs_supported(&self) -> bool {
        self.kargs.is_some()
    }

    // for debugging
    pub fn kargs_header_json(&self) -> Result<Vec<u8>> {
        let mut ret =
            serde_json::to_vec_pretty(&self.kargs).context("failed to serialize kargs header")?;
        ret.push(b'\n');
        Ok(ret)
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
        self.initrd.write(file)?;
        if let Some(kargs) = &self.kargs {
            kargs.write(file)?;
        }
        Ok(())
    }

    pub fn stream(&self, input: &mut File, writer: &mut (impl Write + ?Sized)) -> Result<()> {
        let initrd_region = self.initrd.region()?;
        let mut regions = vec![&initrd_region];
        if let Some(kargs) = &self.kargs {
            regions.extend(kargs.regions.iter())
        }
        regions.stream(input, writer)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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
            32768 - COREOS_INITRD_HEADER_SIZE - COREOS_KARG_EMBED_AREA_HEADER_SIZE,
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

#[derive(Debug, Serialize)]
struct InitrdEmbedArea {
    // region.contents is kept zero-length; region is cloned upon writing
    #[serde(flatten)]
    region: Region,
    #[serde(skip)]
    initrd: Initrd,
}

impl InitrdEmbedArea {
    pub fn for_iso(iso: &mut IsoFs) -> Result<Self> {
        let f = iso
            .get_path(COREOS_INITRD_EMBED_PATH)
            .context("finding initrd embed area")?
            .try_into_file()?;
        // read (checks offset/length as a side effect)
        let mut region = Region::read(iso.as_file()?, f.address.as_offset(), f.length as usize)
            .context("reading initrd embed area")?;
        let initrd = if region.contents.iter().any(|v| *v != 0) {
            Initrd::from_reader(&*region.contents).context("decoding initrd embed area")?
        } else {
            Initrd::default()
        };
        // free up the memory; we won't need it
        region.contents = Vec::new();
        Ok(Self { region, initrd })
    }

    pub fn initrd(&self) -> &Initrd {
        &self.initrd
    }

    pub fn initrd_mut(&mut self) -> &mut Initrd {
        self.region.modified = true;
        &mut self.initrd
    }

    pub fn write(&self, file: &mut File) -> Result<()> {
        self.region()?.write(file)
    }

    pub fn region(&self) -> Result<Region> {
        // taking &mut self for the deferred update to self.region would
        // require too many other methods to do the same, so clone the
        // region and return that
        let mut region = self.region.clone();
        let capacity = region.length;
        let mut data = if !self.initrd().is_empty() {
            self.initrd().to_bytes()?
        } else {
            Vec::new()
        };
        if data.len() > capacity {
            bail!(
                "Compressed initramfs is too large: {} > {}",
                data.len(),
                capacity
            )
        }
        data.extend(repeat(0).take(capacity - data.len()));
        region.contents = data;
        Ok(region)
    }
}

// only for miniso generation
pub(super) fn set_default_kargs(iso: &mut IsoFs, default: String) -> Result<()> {
    let mut kargs_info = KargEmbedInfo::for_iso(iso)?.context(
        // should be impossible; we only support new-style CoreOS ISOs with kargs.json
        "minimal ISO does not have kargs.json; please report this as a bug",
    )?;

    // NB: We don't need to update the length for this; it's a fixed property of the kargs files.
    // (Though its original value did depend on the original default kargs at build time.)
    kargs_info.default = default;
    kargs_info.update_iso(iso)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::copy;

    use tempfile::tempfile;
    use xz2::read::XzDecoder;

    fn open_iso_file() -> File {
        let iso_bytes: &[u8] = include_bytes!("../../fixtures/iso/embed-areas-2021-09.iso.xz");
        let mut decoder = XzDecoder::new(iso_bytes);
        let mut iso_file = tempfile().unwrap();
        copy(&mut decoder, &mut iso_file).unwrap();
        iso_file
    }

    #[test]
    fn test_initrd_embed_area() {
        let mut iso_file = open_iso_file();
        // normal read
        let mut iso = IsoFs::from_file(iso_file.try_clone().unwrap()).unwrap();
        let area = InitrdEmbedArea::for_iso(&mut iso).unwrap();
        assert_eq!(area.region.offset, 102400);
        assert_eq!(area.region.length, 262144);
        // missing embed area
        iso_file.seek(SeekFrom::Start(65903)).unwrap();
        iso_file.write_all(b"Z").unwrap();
        let mut iso = IsoFs::from_file(iso_file).unwrap();
        InitrdEmbedArea::for_iso(&mut iso).unwrap_err();
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
