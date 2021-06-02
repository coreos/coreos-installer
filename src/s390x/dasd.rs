// Copyright 2020 CoreOS, Inc.
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
use gptman::{GPTPartitionEntry, GPT};
use std::fs::File;
use std::io::{self, copy, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::path::Path;

use crate::blockdev::SavedPartitions;
use crate::io::{copy_exactly_n, BUFFER_SIZE};
use crate::s390x::eckd::*;
use crate::s390x::fba::*;

/////////////////////////////////////////////////////////////////////////////
// IBM DASD Support
/////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) struct Range {
    pub in_offset: u64,
    pub out_offset: u64,
    pub length: u64,
}

/// There are 3 types of DASD devices:
///   - ECKD (Extended Count Key Data) - is regular DASD of type 3390
///   - FBA (Fixed Block Access) - is used for emulated device that represents a real SCSI device
///   - Virt - ECKD on LPAR/zKVM as virtio-device
/// Only ECKD disks require `dasdfmt, fdasd` linux tools to be configured.
enum DasdType {
    Eckd,
    Fba,
    Virt,
}

fn get_dasd_type<P: AsRef<Path>>(device: P) -> Result<DasdType> {
    let device = device.as_ref();
    let device = device
        .canonicalize()
        .with_context(|| format!("getting absolute path to {}", device.display()))?
        .file_name()
        .with_context(|| format!("getting name of {}", device.display()))?
        .to_string_lossy()
        .to_string();
    if device.starts_with("vd") {
        return Ok(DasdType::Virt);
    }
    let devtype_path = format!("/sys/class/block/{}/device/devtype", device);
    let devtype_str = std::fs::read_to_string(&devtype_path)
        .with_context(|| format!("reading {}", devtype_path))?;
    let devtype = match devtype_str.starts_with("3390/") {
        true => DasdType::Eckd,
        false => DasdType::Fba,
    };
    Ok(devtype)
}

pub fn prepare_dasd(dasd: &str) -> Result<()> {
    match get_dasd_type(dasd)? {
        DasdType::Eckd => eckd_prepare(dasd),
        DasdType::Fba | DasdType::Virt => Ok(()),
    }
}

/// Returns expected sector size if this is a DASD we'll format later,
/// or None if the caller should use get_sector_size()
pub fn dasd_try_get_sector_size(dasd: &str) -> Result<Option<NonZeroU32>> {
    match get_dasd_type(dasd)? {
        DasdType::Eckd => eckd_try_get_sector_size(dasd),
        DasdType::Fba | DasdType::Virt => Ok(None),
    }
}

pub fn image_copy_s390x(
    first_mb: &[u8],
    source: &mut dyn Read,
    dest_file: &mut File,
    dest_path: &Path,
    _saved: Option<&SavedPartitions>,
) -> Result<()> {
    let ranges = match get_dasd_type(dest_path)? {
        DasdType::Fba => fba_make_partitions(&dest_path.to_string_lossy(), dest_file, first_mb)?,
        DasdType::Eckd | DasdType::Virt => {
            eckd_make_partitions(&dest_path.to_string_lossy(), dest_file, first_mb)?
        }
    };

    // copy each partition
    eprintln!("Installing to {}", dest_path.display());
    let mut buf = [0u8; 1024 * 1024];
    // there shouldn't be any partition data in the first MiB, so don't
    // worry about copying first_mb
    let mut cursor: u64 = 1024 * 1024;
    // amortize write overhead; the decompressor will produce bytes in
    // whatever chunk size it chooses
    let mut dest = BufWriter::with_capacity(BUFFER_SIZE, dest_file);
    let sink = &mut io::sink();
    for range in ranges.iter() {
        if range.in_offset < cursor {
            bail!(
                "found partition at {} when current stream location is {}",
                range.in_offset,
                cursor
            );
        }
        if range.in_offset > cursor {
            copy_exactly_n(source, sink, range.in_offset - cursor, &mut buf)
                .context("sinking input data")?;
            cursor = range.in_offset;
        }
        dest.seek(SeekFrom::Start(range.out_offset))
            .context("seeking output")?;
        copy_exactly_n(source, &mut dest, range.length, &mut buf).context("copying partition")?;
        cursor += range.length;
    }

    // close out the stream
    copy(source, sink).context("reading remainder of stream")?;
    dest.flush().context("flushing data to disk")?;

    Ok(())
}

pub(crate) fn partitions_from_gpt_header(
    bytes_per_block: u64,
    header: &[u8],
) -> Result<Vec<GPTPartitionEntry>> {
    let gpt = GPT::read_from(&mut Cursor::new(header), bytes_per_block)
        .context("reading GPT of source image")?;
    let mut partitions = gpt
        .iter()
        .filter(|(_, pt)| pt.is_used())
        .into_iter()
        .map(|(_, pt)| pt.clone())
        .collect::<Vec<GPTPartitionEntry>>();
    if partitions.is_empty() {
        bail!("source image has no partitions");
    }
    // partitions should be in offset order, but just to be sure
    partitions.sort_unstable_by_key(|r| r.starting_lba);
    Ok(partitions)
}
