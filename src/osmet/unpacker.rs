// Copyright 2020 Red Hat, Inc.
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

use std::convert::{TryFrom, TryInto};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, copy, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{bail, Context, Error, Result};
use openssl::hash::{Hasher, MessageDigest};
use xz2::read::XzDecoder;

use super::*;

/// Path to OSTree repo of sysroot.
const SYSROOT_OSTREE_REPO: &str = "/sysroot/ostree/repo";

pub struct OsmetUnpacker {
    thread_handle: Option<thread::JoinHandle<Result<()>>>,
    reader: pipe::PipeReader,
    length: u64,
}

impl OsmetUnpacker {
    pub fn new(osmet: &Path, repo: &Path) -> Result<Self> {
        let (_, osmet, xzpacked_image) = osmet_file_read(&osmet)?;
        Ok(Self::new_impl(osmet, xzpacked_image, repo))
    }

    pub fn new_from_sysroot(osmet: &Path) -> Result<Self> {
        let (_, osmet, xzpacked_image) = osmet_file_read(&osmet)?;
        Ok(Self::new_impl(
            osmet,
            xzpacked_image,
            Path::new(SYSROOT_OSTREE_REPO),
        ))
    }

    fn new_impl(osmet: Osmet, packed_image: impl Read + Send + 'static, repo: &Path) -> Self {
        let (reader, writer) = pipe::pipe();

        let length = osmet.size;
        let repo = repo.to_owned();
        let thread_handle = Some(thread::spawn(move || -> Result<()> {
            osmet_unpack_to_writer(osmet, packed_image, repo, writer)
        }));

        Self {
            thread_handle,
            reader,
            length,
        }
    }

    pub fn length(&self) -> u64 {
        self.length
    }
}

impl Read for OsmetUnpacker {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        if n == 0 {
            match self
                .thread_handle
                .take()
                .expect("pending thread")
                .join()
                .expect("joining thread")
            {
                Ok(_) => Ok(0),
                Err(e) => Err(io::Error::new(
                    ErrorKind::Other,
                    format!("while unpacking: {}", e),
                )),
            }
        } else {
            Ok(n)
        }
    }
}

pub(super) fn get_unpacked_image_digest(
    xzpacked_image: &mut File,
    partitions: &[OsmetPartition],
    root: &Mount,
) -> Result<(Sha256Digest, u64)> {
    let mut hasher = Hasher::new(MessageDigest::sha256()).context("creating SHA256 hasher")?;
    let repo = root.mountpoint().join("ostree/repo");
    let mut packed_image = XzDecoder::new(xzpacked_image);
    let n = write_unpacked_image(&mut packed_image, &mut hasher, &partitions, &repo)?;
    Ok((hasher.try_into()?, n))
}

struct WriteHasher<W: Write> {
    writer: W,
    hasher: Hasher,
}

impl<W: Write> Write for WriteHasher<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let n = self.writer.write(buf)?;
        self.hasher.write_all(&buf[..n])?;

        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.hasher.flush()?;
        Ok(())
    }
}

impl<W: Write> TryFrom<WriteHasher<W>> for Sha256Digest {
    type Error = Error;

    fn try_from(wrapper: WriteHasher<W>) -> std::result::Result<Self, Self::Error> {
        Sha256Digest::try_from(wrapper.hasher)
    }
}

fn osmet_unpack_to_writer(
    osmet: Osmet,
    mut packed_image: impl Read,
    repo: PathBuf,
    writer: impl Write,
) -> Result<()> {
    let hasher = Hasher::new(MessageDigest::sha256()).context("creating SHA256 hasher")?;

    let mut w = WriteHasher { writer, hasher };

    let n = write_unpacked_image(&mut packed_image, &mut w, &osmet.partitions, &repo)?;
    if n != osmet.size {
        bail!("wrote {} bytes but expected {}", n, osmet.size);
    }

    let final_checksum: Sha256Digest = w.try_into()?;
    if final_checksum != osmet.checksum {
        bail!(
            "expected final checksum {:?}, but got {:?}",
            osmet.checksum,
            final_checksum
        );
    }

    Ok(())
}

fn write_unpacked_image(
    packed_image: &mut impl Read,
    w: &mut impl Write,
    partitions: &[OsmetPartition],
    repo: &Path,
) -> Result<u64> {
    let mut buf = [0u8; 8192];

    // start streaming writes to device, interspersing OSTree objects
    let mut cursor: u64 = 0;
    for partition in partitions {
        assert!(partition.start_offset >= cursor);
        cursor += copy_exactly_n(packed_image, w, partition.start_offset - cursor, &mut buf)?;
        cursor += write_partition(w, partition, packed_image, repo, &mut buf)?;
    }

    // and copy the rest
    cursor += copy(packed_image, w)?;

    Ok(cursor)
}

fn write_partition(
    w: &mut impl Write,
    partition: &OsmetPartition,
    packed_image: &mut impl Read,
    ostree_repo: &Path,
    buf: &mut [u8],
) -> Result<u64> {
    // Set up a reusable buffer for building object paths instead of re-allocating each time. It's
    // easier to maintain it as a Vec<u8> than a PathBuf so we can just use e.g. `write!()`.
    let mut object_pathbuf = {
        let mut repo = Path::new(ostree_repo).to_path_buf();
        repo.push("objects");
        repo.into_os_string().into_vec()
    };
    object_pathbuf.push(b'/');
    let object_pathbuf_n = object_pathbuf.len();

    let mut cursor = partition.start_offset;
    for mapping in partition.mappings.iter() {
        let extent_start = mapping.extent.physical + partition.start_offset;
        assert!(extent_start >= cursor);
        if cursor < extent_start {
            cursor += copy_exactly_n(packed_image, w, extent_start - cursor, buf)?;
        }

        checksum_to_object_path(&mapping.object, &mut object_pathbuf)?;
        cursor += write_partition_mapping(
            &mapping.extent,
            Path::new(OsStr::from_bytes(object_pathbuf.as_slice())),
            w,
            buf,
        )?;
        object_pathbuf.truncate(object_pathbuf_n);
    }

    // and copy to the rest of the partition
    assert!(partition.end_offset >= cursor);
    cursor += copy_exactly_n(packed_image, w, partition.end_offset - cursor, buf)?;

    // subtract back the partition offset here so we only return the actual size of the partition
    Ok(cursor - partition.start_offset)
}

fn write_partition_mapping(
    extent: &Extent,
    object: &Path,
    w: &mut impl Write,
    buf: &mut [u8],
) -> Result<u64> {
    // really, we should be e.g. caching the last N used objects here as open fds so we don't
    // re-open them each time; in practice we don't really encounter much fragmentation, so we can
    // afford to be lazy and keep the code simpler
    let mut object = OpenOptions::new()
        .read(true)
        .open(object)
        .with_context(|| format!("opening {:?}", object))?;

    let mut objlen = object
        .metadata()
        .with_context(|| format!("getting metadata for {:?}", object))?
        .len();

    if extent.logical > 0 {
        object.seek(SeekFrom::Start(extent.logical))?;
        objlen -= extent.logical;
    }

    let mut n = 0;
    if objlen < extent.length {
        n += copy_exactly_n(&mut object, w, objlen, buf)?;
        n += copy_exactly_n(&mut io::repeat(0), w, extent.length - objlen, buf)?;
    } else {
        n += copy_exactly_n(&mut object, w, extent.length, buf)?;
    }

    Ok(n)
}
