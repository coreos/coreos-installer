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

use byte_unit::Byte;
use flate2::read::GzDecoder;
use progress_streams::ProgressReader;
use std::fs::File;
use std::io::{copy, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::time::{Duration, Instant};
use xz2::read::XzDecoder;

use crate::errors::*;
use crate::source::*;
use crate::verify::*;

/// Copy the image to disk and verify its signature.
pub fn write_image(source: &mut ImageSource, dest: &mut File, decompress: bool) -> Result<()> {
    // wrap source for GPG verification
    let mut verify_reader: Box<dyn Read> = {
        if let Some(signature) = source.signature.as_ref() {
            Box::new(GpgReader::new(&mut source.reader, signature)?)
        } else {
            Box::new(&mut source.reader)
        }
    };

    // wrap again for progress reporting
    let artifact_type = source.artifact_type.clone();
    let have_length = source.length_hint.is_some();
    let length_hint = source.length_hint.unwrap_or(0);
    let mut position: u64 = 0;
    let mut last_report = Instant::now();
    let mut progress_reader = ProgressReader::new(&mut verify_reader, |progress: usize| {
        position += progress as u64;
        if last_report.elapsed() >= Duration::from_secs(1)
            || (have_length && position == length_hint)
        {
            last_report = Instant::now();
            if have_length {
                eprint!(
                    "> Read {} {}/{} ({}%)   \r",
                    &artifact_type,
                    format_bytes(position),
                    format_bytes(length_hint),
                    100 * position / length_hint
                );
            } else {
                eprint!("> Read {} {}   \r", &artifact_type, format_bytes(position));
            }
            let _ = std::io::stdout().flush();
        }
    });

    // Wrap in a BufReader so we can peek at the first few bytes for
    // format sniffing.  Don't trust the content-type since the server
    // may not be configured correctly, or the file might be local.
    // Then wrap in a reader for decompression.
    let mut buf_reader = BufReader::new(&mut progress_reader);
    let mut decompress_reader: Box<dyn Read> = {
        let sniff = buf_reader.fill_buf().chain_err(|| "sniffing input")?;
        // verify default buffer size >= the largest magic number we might
        // care about
        assert!(sniff.len() >= 8);
        if !decompress {
            Box::new(buf_reader)
        } else if &sniff[0..2] == b"\x1f\x8b" {
            Box::new(GzDecoder::new(buf_reader))
        } else if &sniff[0..6] == b"\xfd7zXZ\x00" {
            Box::new(XzDecoder::new(buf_reader))
        } else {
            Box::new(buf_reader)
        }
    };

    // Cache the first MiB of input and write zeroes instead.  This ensures
    // that the disk image can't be used accidentally before its GPG signature
    // is verified.
    let mut first_mb: [u8; 1024 * 1024] = [0; 1024 * 1024];
    dest.write_all(&first_mb)
        .chain_err(|| "clearing first MiB of disk")?;
    decompress_reader
        .read_exact(&mut first_mb)
        .chain_err(|| "decoding first MiB of image")?;

    // do the rest of the copy
    // This physically writes any runs of zeroes, rather than sparsifying,
    // but sparsifying is unsafe.  We can't trust that all runs of zeroes in
    // the image represent unallocated blocks, so we must ensure that zero
    // blocks are actually stored as zeroes to avoid image corruption.
    // Discard is insufficient for this: even if our discard request
    // succeeds, discard is not guaranteed to zero blocks (see kernel
    // commits 98262f2762f0 and 48920ff2a5a9).  Ideally we could use
    // BLKZEROOUT to perform hardware-accelerated zeroing and then
    // sparse-copy the image, falling back to non-sparse copy if hardware
    // acceleration is unavailable.  But BLKZEROOUT doesn't support
    // BLKDEV_ZERO_NOFALLBACK, so we'd risk gigabytes of redundant I/O.
    copy(&mut decompress_reader, dest).chain_err(|| "decoding and writing image")?;

    // verify_reader has now checked the signature, so fill in the first MiB
    dest.seek(SeekFrom::Start(0))
        .chain_err(|| "seeking to start of disk")?;
    dest.write_all(&first_mb)
        .chain_err(|| "writing to first MiB of disk")?;

    // flush
    dest.flush().chain_err(|| "flushing data to disk")?;
    dest.sync_all().chain_err(|| "syncing data to disk")?;

    // log final newline
    eprintln!();

    Ok(())
}

/// Format a size in bytes.
fn format_bytes(count: u64) -> String {
    Byte::from_bytes(count.into())
        .get_appropriate_unit(true)
        .format(1)
}
