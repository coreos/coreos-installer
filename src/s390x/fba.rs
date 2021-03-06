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

use anyhow::{Context, Result};
use mbrman::{MBRPartitionEntry, CHS, MBR};
use std::convert::TryInto;
use std::fs::File;

use crate::blockdev::get_sector_size;
use crate::s390x::dasd::{partitions_from_gpt_header, Range};

pub(crate) fn fba_make_partitions(
    dasd: &str,
    device: &mut File,
    first_mb: &[u8],
) -> Result<Vec<Range>> {
    let bytes_per_block = get_sector_size(&device)?.get();
    let partitions = partitions_from_gpt_header(bytes_per_block as u64, first_mb)?;
    let mut ranges = Vec::new();
    let mut mbr = MBR::new_from(device, bytes_per_block, rand::random())
        .with_context(|| format!("creating new partition table for {}", dasd))?;

    for (idx, pt) in partitions.iter().enumerate() {
        let blocks = pt.ending_lba - pt.starting_lba + 1;
        let offset = pt.starting_lba * bytes_per_block as u64;
        ranges.push(Range {
            in_offset: offset,
            out_offset: offset,
            length: blocks * bytes_per_block as u64,
        });
        mbr[idx + 1] = MBRPartitionEntry {
            boot: false,
            first_chs: CHS::empty(),
            sys: 0x83, // MBR_LINUX_DATA_PARTITION
            last_chs: CHS::empty(),
            starting_lba: pt.starting_lba.try_into().with_context(|| {
                format!(
                    "malformed image: pt #{} starting lba is {}",
                    idx + 1,
                    pt.starting_lba
                )
            })?,
            sectors: blocks
                .try_into()
                .with_context(|| format!("malformed image: pt #{} blocks: {}", idx + 1, blocks))?,
        };
    }
    mbr.write_into(device)
        .with_context(|| format!("writing partition table to {}", dasd))?;
    Ok(ranges)
}
