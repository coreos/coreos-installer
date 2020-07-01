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

use error_chain::bail;
use std::io::{ErrorKind, Read, Write};

use crate::errors::*;

/// This is like `std::io:copy()`, but limits the number of bytes copied over. The `Read` trait has
/// `take()`, but that takes ownership of the reader. We also take a buf to avoid re-initializing a
/// block each time (std::io::copy() gets around this by using MaybeUninit, but that requires using
/// nightly and unsafe functions).
pub fn copy_n(
    reader: &mut impl Read,
    writer: &mut impl Write,
    mut n: u64,
    buf: &mut [u8],
) -> Result<u64> {
    let mut written = 0;
    loop {
        if n == 0 {
            return Ok(written);
        }
        let bufn = if n < (buf.len() as u64) {
            &mut buf[..n as usize]
        } else {
            &mut buf[..]
        };
        let len = match reader.read(bufn) {
            Ok(0) => return Ok(written),
            Ok(len) => len,
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        assert!(len as u64 <= n);
        writer.write_all(&bufn[..len])?;
        written += len as u64;
        n -= len as u64;
    }
}

/// This is like `copy_n()` but errors if the number of bytes copied is less than expected.
pub fn copy_exactly_n(
    reader: &mut impl Read,
    writer: &mut impl Write,
    n: u64,
    buf: &mut [u8],
) -> Result<u64> {
    let bytes_copied = copy_n(reader, writer, n, buf)?;
    if bytes_copied != n {
        bail!(
            "expected to copy {} bytes but instead copied {} bytes",
            n,
            bytes_copied
        );
    }
    Ok(n)
}
