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

use std::collections::HashMap;
use std::fs::File;
use std::io::{copy, Read, Seek, SeekFrom, Write};

use anyhow::{bail, Context, Result};
use bincode::Options;
use clap::crate_version;
use serde::{Deserialize, Serialize};
use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

use crate::io::*;
use crate::iso9660;

/// Magic header value for miniso data file.
const HEADER_MAGIC: [u8; 8] = *b"MINISO\0\0";

/// Basic versioning. Used as a safety check that we're unpacking a miniso data file we understand.
/// Bump this when making changes to the format.
const HEADER_VERSION: u32 = 1;

/// Maximum size of miniso data file we'll agree to deserialize. FCOS is currently
/// at 2892 bytes, so this is generous.
const DATA_MAX_SIZE: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, Debug)]
struct Table {
    entries: Vec<TableEntry>,
}

impl Table {
    fn new(
        full_files: &HashMap<String, iso9660::File>,
        minimal_files: &HashMap<String, iso9660::File>,
    ) -> Result<(Self, usize)> {
        let mut entries: Vec<TableEntry> = Vec::new();
        for (path, minimal_entry) in minimal_files {
            let full_entry = full_files
                .get(path)
                .with_context(|| format!("missing minimal file {} in full ISO", path))?;
            if full_entry.length != minimal_entry.length {
                bail!(
                    "File {} has different lengths in full and minimal ISOs",
                    path
                );
            }
            entries.push(TableEntry {
                minimal: minimal_entry.address,
                full: full_entry.address,
                length: full_entry.length,
            });
        }

        entries.sort_by_key(|e| e.minimal.as_sector());
        // drop zero-length files (which can overlap with other files) and
        // duplicate entries (hardlinks), and calculate how many there were
        // for reporting
        let size = entries.len();
        entries = entries.drain(..).filter(|e| e.length > 0).collect();
        entries.dedup();
        let extraneous = size - entries.len();
        let table = Table { entries };
        table.validate().context("validating table")?;
        Ok((table, extraneous))
    }

    fn validate(&self) -> Result<()> {
        let n = self.entries.len();
        if n == 0 {
            bail!("table is empty; ISOs have no files in common?");
        }
        for (e, next_e) in self.entries[..n - 1].iter().zip(self.entries[1..n].iter()) {
            if e.minimal.as_offset() + e.length as u64 > next_e.minimal.as_offset() {
                bail!(
                    "Files at offsets {} and {} overlap",
                    e.minimal.as_offset(),
                    next_e.minimal.as_offset(),
                );
            }
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct TableEntry {
    minimal: iso9660::Address,
    full: iso9660::Address,
    length: u32,
}

// Version-agnostic header. Frozen.
#[derive(Serialize, Deserialize, Debug)]
struct Header {
    magic: [u8; 8],
    version: u32,
    /// For informational purposes only.
    app_version: String,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            magic: HEADER_MAGIC,
            version: HEADER_VERSION,
            app_version: crate_version!().into(),
        }
    }
}

impl Header {
    pub fn validate(&self) -> Result<()> {
        if self.magic != HEADER_MAGIC {
            bail!("not a miniso file!");
        }
        if self.version != HEADER_VERSION {
            bail!(
                "incompatible miniso file version: {} vs {} (created by {})",
                HEADER_VERSION,
                self.version,
                self.app_version,
            );
        }
        Ok(())
    }
}

// Version-specific payload. Evolvable.
#[derive(Serialize, Deserialize, Debug)]
pub struct Data {
    table: Table,
    digest: Sha256Digest,
    xzpacked: Vec<u8>,
}

impl Data {
    pub fn xzpack(
        miniso: &mut File,
        full_files: &HashMap<String, iso9660::File>,
        minimal_files: &HashMap<String, iso9660::File>,
    ) -> Result<(Self, usize, u64, u64, u64)> {
        let (table, extraneous) = Table::new(full_files, minimal_files)?;

        // A `ReadHasher` here would let us wrap the miniso so we calculate the digest as we read.
        let digest = Sha256Digest::from_file(miniso)?;
        let mut offset = miniso
            .seek(SeekFrom::Start(0))
            .context("seeking back to miniso start")?;

        let mut xzw = XzEncoder::new(Vec::new(), 9);
        let mut buf = [0u8; BUFFER_SIZE];
        let mut skipped: u64 = 0;
        for entry in &table.entries {
            let addr: u64 = entry.minimal.as_offset();
            assert!(offset <= addr);
            if addr > offset {
                copy_exactly_n(miniso, &mut xzw, addr - offset, &mut buf).with_context(|| {
                    format!(
                        "copying {} miniso bytes at offset {}",
                        addr - offset,
                        offset
                    )
                })?;
            }
            // I tested trying to be smarter here and rounding to the nearest 2k block so we can
            // skip padding, but zeroes compress so well that it only saved a grand total of 4
            // bytes after xz. So not worth the complexity.
            offset = miniso
                .seek(SeekFrom::Current(entry.length as i64))
                .with_context(|| format!("skipping miniso file at offset {}", addr))?;
            skipped += entry.length as u64;
        }

        copy(miniso, &mut xzw).context("copying remaining miniso bytes")?;

        xzw.try_finish().context("trying to finish xz stream")?;
        let matches = table.entries.len() + extraneous;
        let written = xzw.total_in();
        let written_compressed = xzw.total_out();
        Ok((
            Self {
                table,
                digest,
                xzpacked: xzw.finish().context("finishing xz stream")?,
            },
            matches,
            skipped,
            written,
            written_compressed,
        ))
    }

    pub fn serialize(&self, w: impl Write) -> Result<()> {
        let mut limiter = LimitWriter::new(w, DATA_MAX_SIZE, "data size limit".into());

        let header = Header::default();
        let coder = &mut bincoder();
        coder
            .serialize_into(&mut limiter, &header)
            .context("failed to serialize header")?;
        coder
            .serialize_into(&mut limiter, &self)
            .context("failed to serialize data")?;

        Ok(())
    }

    pub fn deserialize(r: impl Read) -> Result<Self> {
        let mut limiter = LimitReader::new(r, DATA_MAX_SIZE, "data size limit".into());

        let coder = &mut bincoder();
        let header: Header = coder
            .deserialize_from(&mut limiter)
            .context("failed to deserialize header")?;
        header.validate().context("validating header")?;

        let data: Self = coder
            .deserialize_from(&mut limiter)
            .context("failed to deserialize data")?;
        data.table.validate().context("validating table")?;

        Ok(data)
    }

    pub fn unxzpack(&self, fulliso: &mut File, w: impl Write) -> Result<()> {
        let mut xzr = XzDecoder::new(self.xzpacked.as_slice());
        let mut w = WriteHasher::new_sha256(w)?;
        let mut buf = [0u8; BUFFER_SIZE];
        let mut offset = 0;
        for entry in &self.table.entries {
            let minimal_addr = entry.minimal.as_offset();
            let fulliso_addr = entry.full.as_offset();
            if minimal_addr > offset {
                offset += copy_exactly_n(&mut xzr, &mut w, minimal_addr - offset, &mut buf)
                    .with_context(|| {
                        format!(
                            "copying {} packed bytes at offset {}",
                            minimal_addr - offset,
                            offset
                        )
                    })?;
            }
            fulliso
                .seek(SeekFrom::Start(fulliso_addr))
                .with_context(|| format!("seeking to full ISO file at offset {}", fulliso_addr))?;
            offset += copy_exactly_n(fulliso, &mut w, entry.length as u64, &mut buf)
                .with_context(|| format!("copying full ISO file at offset {}", fulliso_addr))?;
        }

        copy(&mut xzr, &mut w).context("copying remaining packed bytes")?;
        let digest = w.try_into()?;
        if self.digest != digest {
            bail!(
                "wrong final digest: expected {}, found {}",
                self.digest.to_hex_string()?,
                digest.to_hex_string()?
            );
        }

        Ok(())
    }
}
