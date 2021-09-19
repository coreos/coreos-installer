// Copyright 2021 Red Hat, Inc.
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

//! Minimal ISO 9660 parser. This is not a comprehensive parser; we only
//! parse out the fields we care about. Extensions such as Rock Ridge
//! and Joliet are not supported.
//!
//! The official specification is not free. The primary reference used
//! for this module is https://wiki.osdev.org/ISO_9660.

// An initial version of this module used the zerocopy crate to try to deserialize directly from
// the on-disk ISO file in with zero copying. It works, but it's non-trivial and the performance
// difference from just copying stuff didn't justify it.

// Many magic numbers corresponding to offsets and lengths have not been const-ified. It should be
// straightforward to see to what they correspond using the referenced linked above.

use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use bytes::{Buf, Bytes};
use serde::Serialize;

use crate::io::BUFFER_SIZE;

// technically the standard supports others, but this is the only one we support
const ISO9660_SECTOR_SIZE: usize = 2048;

#[derive(Debug, Serialize)]
pub struct IsoFs {
    descriptors: Vec<VolumeDescriptor>,
    #[serde(skip_serializing)]
    file: fs::File,
}

impl IsoFs {
    pub fn from_file(mut file: fs::File) -> Result<Self> {
        let descriptors = get_volume_descriptors(&mut file)?;
        Ok(Self { descriptors, file })
    }

    pub fn as_file(&mut self) -> Result<&mut fs::File> {
        self.file
            .seek(SeekFrom::Start(0))
            .context("seeking to start of ISO")?;
        Ok(&mut self.file)
    }

    pub fn get_root_directory(&self) -> Result<Directory> {
        let primary = self
            .get_primary_volume_descriptor()
            .context("getting root directory")?;
        Ok(primary.root.clone())
    }

    pub fn walk(&mut self) -> Result<IsoFsWalkIterator> {
        let root_dir = self.get_root_directory()?;
        let buf = self.list_dir(&root_dir)?;
        Ok(IsoFsWalkIterator {
            iso: &mut self.file,
            parent_dirs: Vec::new(),
            current_dir: Some(buf),
            dirpath: PathBuf::new(),
        })
    }

    /// Returns an iterator over the records of a directory.
    pub fn list_dir(&mut self, dir: &Directory) -> Result<IsoFsIterator> {
        IsoFsIterator::new(&mut self.file, dir)
    }

    /// Returns the record for a specific path.
    pub fn get_path(&mut self, path: &str) -> Result<DirectoryRecord> {
        let root_dir = self.get_root_directory()?;
        let as_path = Path::new(path);
        let mut parent_dir = if let Some(p) = as_path.parent() {
            p
        } else {
            return Ok(DirectoryRecord::Directory(root_dir));
        };
        if parent_dir.has_root() {
            parent_dir = parent_dir
                .strip_prefix("/")
                .with_context(|| format!("making path '{}' relative", path))?;
        }
        let filename = as_path
            .file_name()
            .ok_or_else(|| anyhow!("path {} has no base", path))?;
        let filename = filename.to_str().unwrap(); // `path` is &str

        let mut dir = root_dir;
        for component in parent_dir.components() {
            if let std::path::Component::Normal(c) = component {
                let c = c.to_str().unwrap(); // `path` is &str
                dir = self
                    .get_dir_record(&dir, c)?
                    .ok_or_else(|| anyhow!("intermediate directory {} does not exist", c))?
                    .try_into_dir()
                    .map_err(|_| {
                        anyhow!("component {:?} in path {} is not a directory", c, path)
                    })?;
            } else {
                bail!("path is not canonical: {}", path);
            }
        }

        self.get_dir_record(&dir, filename)?.ok_or_else(|| {
            anyhow!(
                "no record for {} in directory {}",
                filename,
                parent_dir.display()
            )
        })
    }

    /// Returns the record for a specific name in a directory if it exists.
    pub fn get_dir_record(
        &mut self,
        dir: &Directory,
        name: &str,
    ) -> Result<Option<DirectoryRecord>> {
        for record in self
            .list_dir(dir)
            .with_context(|| format!("listing directory {}", dir.name))?
        {
            let record = record?;
            match &record {
                DirectoryRecord::Directory(d) if d.name == name => return Ok(Some(record)),
                DirectoryRecord::File(f) if f.name == name => return Ok(Some(record)),
                _ => continue,
            }
        }
        Ok(None)
    }

    /// Returns a reader for a file record.
    pub fn read_file(&mut self, file: &File) -> Result<impl Read + '_> {
        self.file
            .seek(SeekFrom::Start(file.address))
            .with_context(|| format!("seeking to file {}", file.name))?;
        Ok(BufReader::with_capacity(
            BUFFER_SIZE,
            (&self.file).take(file.length as u64),
        ))
    }

    fn get_primary_volume_descriptor(&self) -> Result<&PrimaryVolumeDescriptor> {
        for d in &self.descriptors {
            if let VolumeDescriptor::Primary(p) = d {
                return Ok(p);
            }
        }
        Err(anyhow!("no primary volume descriptor found in ISO"))
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum VolumeDescriptor {
    Boot(BootVolumeDescriptor),
    Primary(PrimaryVolumeDescriptor),
    Supplementary,
    Unknown { type_id: u8 },
}

#[derive(Debug, Serialize)]
struct BootVolumeDescriptor {
    boot_system_id: String,
    boot_id: String,
}

#[derive(Debug, Serialize)]
struct PrimaryVolumeDescriptor {
    system_id: String,
    volume_id: String,
    root: Directory,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DirectoryRecord {
    Directory(Directory),
    File(File),
}

impl DirectoryRecord {
    pub fn try_into_dir(self) -> Result<Directory> {
        match self {
            Self::Directory(d) => Ok(d),
            Self::File(f) => Err(anyhow!("entry {} is a file", f.name)),
        }
    }

    pub fn try_into_file(self) -> Result<File> {
        match self {
            Self::Directory(f) => Err(anyhow!("entry {} is a directory", f.name)),
            Self::File(f) => Ok(f),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct Directory {
    pub name: String,
    pub address: u64,
    pub length: u32,
}

#[derive(Debug, Serialize, Clone)]
pub struct File {
    pub name: String,
    pub address: u64,
    pub length: u32,
}

/// Reads all the volume descriptors.
fn get_volume_descriptors(f: &mut fs::File) -> Result<Vec<VolumeDescriptor>> {
    const ISO9660_VOLUME_DESCRIPTORS: u64 = 0x10 * (ISO9660_SECTOR_SIZE as u64);
    f.seek(SeekFrom::Start(ISO9660_VOLUME_DESCRIPTORS))
        .context("seeking to volume descriptors")?;

    let mut descriptors: Vec<VolumeDescriptor> = Vec::new();
    while let Some(d) = get_next_volume_descriptor(f)
        .with_context(|| format!("getting volume descriptor #{}", descriptors.len() + 1))?
    {
        descriptors.push(d);
    }

    Ok(descriptors)
}

/// Reads the volume descriptor at cursor and advances to the next one.
fn get_next_volume_descriptor(f: &mut fs::File) -> Result<Option<VolumeDescriptor>> {
    const TYPE_BOOT: u8 = 0;
    const TYPE_PRIMARY: u8 = 1;
    const TYPE_SUPPLEMENTARY: u8 = 2;
    const TYPE_TERMINATOR: u8 = 255;

    let mut buf = vec![0; ISO9660_SECTOR_SIZE];
    f.read_exact(&mut buf)
        .context("reading volume descriptor")?;
    let buf = &mut Bytes::from(buf);

    Ok(match buf.get_u8() {
        TYPE_BOOT => Some(VolumeDescriptor::Boot(BootVolumeDescriptor::parse(buf)?)),
        TYPE_PRIMARY => Some(VolumeDescriptor::Primary(PrimaryVolumeDescriptor::parse(
            buf,
        )?)),
        TYPE_SUPPLEMENTARY => Some(VolumeDescriptor::Supplementary),
        TYPE_TERMINATOR => None,
        t => Some(VolumeDescriptor::Unknown { type_id: t }),
    })
}

impl BootVolumeDescriptor {
    /// Parses boot descriptor at cursor after type field.
    fn parse(buf: &mut Bytes) -> Result<Self> {
        verify_descriptor_header(buf).context("parsing boot descriptor")?;
        Ok(Self {
            boot_system_id: parse_iso9660_string(buf, 32, IsoString::StrA)
                .context("parsing boot system ID")?,
            boot_id: parse_iso9660_string(buf, 32, IsoString::StrA).context("parsing boot ID")?,
        })
    }
}

impl PrimaryVolumeDescriptor {
    /// Parses primary descriptor at cursor after type field.
    fn parse(buf: &mut Bytes) -> Result<Self> {
        verify_descriptor_header(buf).context("parsing primary descriptor")?;
        let system_id =
            parse_iso9660_string(eat(buf, 1), 32, IsoString::StrA).context("parsing system id")?;
        let volume_id = // technically should be StrD, but non-compliance is common
            parse_iso9660_string(buf, 32, IsoString::StrA).context("parsing volume id")?;
        let root = match get_next_directory_record(eat(buf, 156 - 72), 34)? {
            Some(DirectoryRecord::Directory(d)) => d,
            _ => bail!("failed to parse root directory record from primary descriptor"),
        };
        Ok(Self {
            system_id,
            volume_id,
            root,
        })
    }
}

/// Verifies descriptor header at cursor.
fn verify_descriptor_header(buf: &mut Bytes) -> Result<()> {
    const VOLUME_DESCRIPTOR_ID: &[u8] = b"CD001";
    const VOLUME_DESCRIPTOR_VERSION: u8 = 1;

    let id = buf.copy_to_bytes(5);
    if id != VOLUME_DESCRIPTOR_ID {
        bail!("unknown descriptor ID: {:?}", id);
    }

    let version = buf.get_u8();
    if version != VOLUME_DESCRIPTOR_VERSION {
        bail!("unknown descriptor version: {}", version);
    }

    Ok(())
}

pub struct IsoFsIterator {
    dir: Bytes,
    length: u32,
}

impl IsoFsIterator {
    fn new(iso: &mut fs::File, dir: &Directory) -> Result<Self> {
        iso.seek(SeekFrom::Start(dir.address))
            .with_context(|| format!("seeking to directory {}", dir.name))?;

        let mut buf = vec![0; dir.length as usize];
        iso.read_exact(&mut buf)
            .with_context(|| format!("reading directory {}", dir.name))?;

        Ok(Self {
            dir: Bytes::from(buf),
            length: dir.length,
        })
    }
}

impl Iterator for IsoFsIterator {
    type Item = Result<DirectoryRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        get_next_directory_record(&mut self.dir, self.length)
            .context("reading next record")
            .transpose()
    }
}

pub struct IsoFsWalkIterator<'a> {
    iso: &'a mut fs::File,
    parent_dirs: Vec<IsoFsIterator>,
    current_dir: Option<IsoFsIterator>,
    dirpath: PathBuf,
}

impl<'a> Iterator for IsoFsWalkIterator<'a> {
    type Item = Result<(String, DirectoryRecord)>;
    fn next(&mut self) -> Option<Self::Item> {
        self.walk_iterator_next().transpose()
    }
}

impl<'a> IsoFsWalkIterator<'a> {
    // This is simply split out of next() above for easier error-handling
    fn walk_iterator_next(&mut self) -> Result<Option<(String, DirectoryRecord)>> {
        while let Some(ref mut current_dir) = self.current_dir {
            match current_dir.next() {
                Some(Ok(r)) => {
                    // ideally, we'd return a ref and avoid cloning, but there's no way for an
                    // iterator to return a reference to data within itself
                    let mut path = self.dirpath.clone();
                    match r {
                        DirectoryRecord::Directory(ref d) => {
                            if d.name == "." || d.name == ".." {
                                continue;
                            }
                            self.parent_dirs.push(self.current_dir.take().unwrap());
                            self.dirpath.push(&d.name);
                            self.current_dir = Some(IsoFsIterator::new(self.iso, d)?);
                            path.push(&d.name);
                        }
                        DirectoryRecord::File(ref f) => path.push(&f.name),
                    };
                    // paths are all UTF-8
                    return Ok(Some((path.into_os_string().into_string().unwrap(), r)));
                }
                Some(Err(e)) => return Err(e),
                None => {
                    self.current_dir = self.parent_dirs.pop();
                    self.dirpath.pop();
                }
            }
        }
        Ok(None)
    }
}

/// Reads the directory record at cursor and advances to the next one.
fn get_next_directory_record(buf: &mut Bytes, length: u32) -> Result<Option<DirectoryRecord>> {
    if !buf.has_remaining() {
        return Ok(None);
    }

    let mut len = buf.get_u8() as usize;
    if len == 0 {
        let jump = {
            // calculate where we are we in the directory
            let pos = length as usize - buf.remaining();
            // get distance to next 2k-aligned address
            ((pos + ISO9660_SECTOR_SIZE) & !(ISO9660_SECTOR_SIZE - 1)) - pos
        };
        if jump >= buf.remaining() {
            return Ok(None);
        }
        buf.advance(jump);
        len = buf.get_u8() as usize;
    }

    // + 1 because len includes the length of the length byte itself, which we already read
    if buf.remaining() + 1 < len {
        bail!("incomplete directory record; corrupt ISO?");
    }

    let address = (eat(buf, 1).get_u32_le() as u64) * (ISO9660_SECTOR_SIZE as u64);
    let length = eat(buf, 4).get_u32_le();
    let flags = eat(buf, 25 - 14).get_u8();
    let name_length = eat(buf, 32 - 26).get_u8() as usize;
    let name = parse_iso9660_path(buf, name_length).context("parsing record name")?;

    // advance to next record
    eat(buf, len - (33 + name_length));

    if flags & 2 > 0 {
        Ok(Some(DirectoryRecord::Directory(Directory {
            name,
            address,
            length,
        })))
    } else {
        Ok(Some(DirectoryRecord::File(File {
            name,
            address,
            length,
        })))
    }
}

/// Reads a directory record path. This is similar to a regular ISO9660 string, but supports '\0'
/// to mean current directory, and '\1' for the parent directory.
fn parse_iso9660_path(buf: &mut Bytes, len: usize) -> Result<String> {
    if len == 1 && (buf[0] == 0 || buf[0] == 1) {
        let c = buf.get_u8();
        if c == 0 {
            Ok(".".into())
        } else {
            Ok("..".into())
        }
    } else {
        parse_iso9660_string(buf, len, IsoString::File)
    }
}

#[allow(unused)]
enum IsoString {
    StrA,
    StrD,
    File,
}

/// Reads an ISO9660 string.
fn parse_iso9660_string(buf: &mut Bytes, len: usize, kind: IsoString) -> Result<String> {
    // References:
    // https://wiki.osdev.org/ISO_9660#String_format
    // https://github.com/torvalds/linux/blob/ddf21bd8ab984ccaa924f090fc7f515bb6d51414/fs/isofs/dir.c#L17
    const FILE_CHARS: [u8; 17] = *b"!\"%&'()*+,-.:<=>?"; // full file chars set includes D-chars
    const A_CHARS: [u8; 2] = *b";/"; // full A-chars includes file chars set
    if len > buf.remaining() {
        bail!("incomplete string name; corrupt ISO?");
    }
    let mut s = String::with_capacity(len);
    let mut bytes = buf.copy_to_bytes(len);
    if matches!(kind, IsoString::File) {
        if bytes.ends_with(b";1") {
            bytes.truncate(bytes.len() - 2);
        }
        if bytes.ends_with(b".") {
            bytes.truncate(bytes.len() - 1);
        }
    }
    for byte in &bytes {
        #[allow(clippy::if_same_then_else)] // I find it easier to follow this way
        if byte.is_ascii_alphabetic() || byte.is_ascii_digit() || *byte == b'_' || *byte == b' ' {
            s.push(char::from(*byte));
        } else if FILE_CHARS.contains(byte) && matches!(kind, IsoString::File | IsoString::StrA) {
            s.push(char::from(*byte));
        } else if A_CHARS.contains(byte) && matches!(kind, IsoString::StrA) {
            s.push(char::from(*byte));
        } else if A_CHARS.contains(byte) && matches!(kind, IsoString::File) {
            s.push('.'); // this matches what the kernel does
        } else if *byte == 0 {
            break;
        } else {
            bail!("invalid string name {:?}", bytes);
        }
    }
    if matches!(kind, IsoString::StrA | IsoString::StrD) {
        s.truncate(s.trim_end_matches(' ').len());
    }
    Ok(s)
}

fn eat(buf: &mut Bytes, n: usize) -> &mut Bytes {
    buf.advance(n);
    buf
}
