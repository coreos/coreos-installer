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

use error_chain::bail;
use std::fs::File;
use std::io::{self, copy, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::blockdev::SavedPartitions;
use crate::errors::*;
use crate::io::{copy_exactly_n, BUFFER_SIZE};
use crate::s390x::eckd::{
    default_format, is_invalid, low_level_format, make_partitions, partition_ranges,
};

/////////////////////////////////////////////////////////////////////////////
// IBM DASD Support
/////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub(crate) struct Range {
    pub in_offset: u64,
    pub out_offset: u64,
    pub length: u64,
}

pub fn prepare_dasd(device: &str) -> Result<()> {
    low_level_format(device)?;
    if is_invalid(device)? {
        eprintln!("Disk {} is invalid, formatting", device);
        default_format(device)?
    }
    Ok(())
}

pub fn image_copy_s390x(
    first_mb: &[u8],
    source: &mut dyn Read,
    dest_file: &mut File,
    dest_path: &Path,
    _saved: Option<&SavedPartitions>,
) -> Result<()> {
    let (ranges, partitions) = partition_ranges(first_mb, dest_file)?;
    make_partitions(
        dest_path
            .to_str()
            .chain_err(|| format!("couldn't encode path {}", dest_path.display()))?,
        &partitions,
    )?;

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
                .chain_err(|| "sinking input data")?;
            cursor = range.in_offset;
        }
        dest.seek(SeekFrom::Start(range.out_offset))
            .chain_err(|| "seeking output")?;
        copy_exactly_n(source, &mut dest, range.length, &mut buf)
            .chain_err(|| "copying partition")?;
        cursor += range.length;
    }

    // close out the stream
    copy(source, sink).chain_err(|| "reading remainder of stream")?;
    dest.flush().chain_err(|| "flushing data to disk")?;

    Ok(())
}
