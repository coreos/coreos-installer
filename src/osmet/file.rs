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

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use bincode::Options;
use clap::crate_version;
use serde::{Deserialize, Serialize};
use xz2::read::XzDecoder;

use crate::io::BUFFER_SIZE;

use super::*;

/// Magic header value for osmet binary.
const OSMET_FILE_HEADER_MAGIC: [u8; 8] = *b"OSMET\0\0\0";

/// Basic versioning. Used as a safety check that we're unpacking something we understand. Bump
/// this when making changes to the format.
const OSMET_FILE_VERSION: u32 = 1;

/// We currently use bincode for serialization. Note bincode does not support backwards compatible
/// changes well. However we do not currently care about backcompatibility. If that changes, we
/// should change serializer.
#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OsmetFileHeader {
    magic: [u8; 8],
    version: u32,
    /// For informational purposes only.
    app_version: String,
    /// Required sector size of target block device during unpacking.
    pub(super) sector_size: u32,
    pub(super) os_description: String,
    pub(super) os_architecture: String,
}

impl OsmetFileHeader {
    pub(super) fn new(sector_size: u32, os_description: &str) -> Self {
        Self {
            magic: OSMET_FILE_HEADER_MAGIC,
            version: OSMET_FILE_VERSION,
            app_version: crate_version!().into(),
            sector_size,
            os_description: os_description.into(),
            // There's an assumption here that the OS we're packing is for the same
            // architecture on which we're running. This holds, because packing is done by cosa,
            // which today doesn't support cross-building. But the osmet format and algorithm
            // itself actually doesn't care about the target architecture. In the future, a more
            // correct approach is to read this directly from the e.g. coreos-assembler.basearch
            // in the commit metadata on the source disk.
            os_architecture: nix::sys::utsname::uname().machine().into(),
        }
    }
}

pub(super) fn osmet_file_write(
    path: &Path,
    header: OsmetFileHeader,
    osmet: Osmet,
    mut xzpacked_image: File,
) -> Result<()> {
    validate_osmet(&osmet).context("validating before writing")?;

    // would be nice to opportunistically do open(O_TMPFILE) then linkat here, but the tempfile API
    // doesn't provide that API: https://github.com/Stebalien/tempfile/pull/31
    let mut f = BufWriter::with_capacity(
        BUFFER_SIZE,
        tempfile::Builder::new()
            .prefix("coreos-installer-osmet")
            .suffix(".partial")
            .tempfile_in(path.parent().unwrap())?,
    );

    bincoder()
        .serialize_into(&mut f, &header)
        .context("failed to serialize osmet file header")?;
    bincoder()
        .serialize_into(&mut f, &osmet)
        .context("failed to serialize osmet")?;

    // and followed by the xz-compressed packed image
    copy(&mut xzpacked_image, &mut f)?;

    f.into_inner()
        .context("failed to flush write buffer")?
        .persist(path)
        .with_context(|| format!("failed to persist tempfile to {:?}", path))?;

    Ok(())
}

/// Reads in the header, and does some basic sanity checking.
fn read_and_check_header(mut f: &mut impl Read) -> Result<OsmetFileHeader> {
    let header: OsmetFileHeader = bincoder()
        .deserialize_from(&mut f)
        .context("failed to deserialize osmet file")?;
    if header.magic != OSMET_FILE_HEADER_MAGIC {
        bail!("not an OSMET file!");
    }
    if header.version != OSMET_FILE_VERSION {
        bail!("incompatible OSMET file version {}", header.version);
    }

    Ok(header)
}

pub(super) fn osmet_file_read_header(path: &Path) -> Result<OsmetFileHeader> {
    let mut f = BufReader::with_capacity(
        BUFFER_SIZE,
        OpenOptions::new()
            .read(true)
            .open(path)
            .with_context(|| format!("opening {:?}", path))?,
    );

    read_and_check_header(&mut f)
}

pub(super) fn osmet_file_read(path: &Path) -> Result<(OsmetFileHeader, Osmet, impl Read + Send)> {
    let mut f = BufReader::with_capacity(
        BUFFER_SIZE,
        OpenOptions::new()
            .read(true)
            .open(path)
            .with_context(|| format!("opening {:?}", path))?,
    );

    let header = read_and_check_header(&mut f)?;
    let osmet: Osmet = bincoder()
        .deserialize_from(&mut f)
        .context("failed to deserialize osmet file")?;

    validate_osmet(&osmet).context("validating after reading")?;
    Ok((header, osmet, XzDecoder::new(f)))
}

fn validate_osmet(osmet: &Osmet) -> Result<()> {
    if osmet.partitions.is_empty() {
        bail!("OSMET file has no partitions!");
    }

    // sanity-check partitions and mappings are in canonical form
    let mut cursor: u64 = 0;
    for (i, partition) in osmet.partitions.iter().enumerate() {
        if cursor > partition.start_offset {
            bail!(
                "cursor past partition start: {} vs {}",
                cursor,
                partition.start_offset
            );
        }
        cursor = cursor
            .checked_add(
                verify_canonical(&partition.mappings)
                    .with_context(|| format!("partition {}", i))?,
            )
            .ok_or_else(|| anyhow!("overflow after partition {}", i))?;
        if cursor > partition.end_offset {
            bail!(
                "cursor past partition end: {} vs {}",
                cursor,
                partition.end_offset
            );
        }
        cursor = partition.end_offset;
    }

    Ok(())
}

fn verify_canonical(mappings: &[Mapping]) -> Result<u64> {
    let mut cursor: u64 = 0;
    for (i, mapping) in mappings.iter().enumerate() {
        if cursor > mapping.extent.physical {
            bail!(
                "cursor past mapping start: {} vs {}",
                cursor,
                mapping.extent.physical
            );
        }
        cursor = mapping
            .extent
            .physical
            .checked_add(mapping.extent.length)
            .ok_or_else(|| anyhow!("overflow after mapping {}", i))?;
    }

    Ok(cursor)
}

fn bincoder() -> impl bincode::Options {
    bincode::options()
        .allow_trailing_bytes()
        // make the defaults explicit
        .with_no_limit()
        .with_little_endian()
        .with_varint_encoding()
}
