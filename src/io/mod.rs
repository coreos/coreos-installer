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

use anyhow::{bail, Result};
use bincode::Options;
use std::io::{ErrorKind, Read, Write};

mod bls;
mod compress;
mod hash;
mod ignition;
mod initrd;
mod limit;
mod peek;
mod tee;
mod verify;
mod xz;
mod zstd;

pub use self::bls::*;
pub use self::compress::*;
pub use self::hash::*;
pub use self::ignition::*;
pub use self::initrd::*;
pub use self::limit::*;
pub use self::peek::*;
pub use self::tee::*;
pub use self::verify::*;
pub use self::xz::*;
pub use self::zstd::*;

// The default BufReader/BufWriter buffer size is 8 KiB, which isn't large
// enough to fully amortize system call overhead.
// https://github.com/rust-lang/rust/issues/49921
// https://github.com/coreutils/coreutils/blob/6a3d2883/src/ioblksize.h
pub const BUFFER_SIZE: usize = 256 * 1024;

/// This is like `std::io:copy()`, but limits the number of bytes copied over. The `Read` trait has
/// `take()`, but that takes ownership of the reader. We also take a buf to avoid re-initializing a
/// block each time (std::io::copy() gets around this by using MaybeUninit, but that requires using
/// nightly and unsafe functions).
pub fn copy_n(
    reader: &mut (impl Read + ?Sized),
    writer: &mut (impl Write + ?Sized),
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
            &mut *buf
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
    reader: &mut (impl Read + ?Sized),
    writer: &mut (impl Write + ?Sized),
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

/// Provides uniform bincode options for all our serialization operations.
pub fn bincoder() -> impl bincode::Options {
    bincode::options()
        .allow_trailing_bytes()
        // make the defaults explicit
        .with_no_limit()
        .with_little_endian()
        .with_varint_encoding()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_n() {
        let mut sink = std::io::sink();
        let mut buf = [0u8; 50];

        let data = [0u8; 30];
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 0, &mut buf).unwrap(),
            0
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 1, &mut buf).unwrap(),
            1
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 29, &mut buf).unwrap(),
            29
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 30, &mut buf).unwrap(),
            30
        );
        assert_eq!(copy_n(&mut &data[..], &mut sink, 31, &mut buf).unwrap(), 30);
        assert_eq!(copy_n(&mut &data[..], &mut sink, 49, &mut buf).unwrap(), 30);
        assert_eq!(copy_n(&mut &data[..], &mut sink, 50, &mut buf).unwrap(), 30);
        assert_eq!(copy_n(&mut &data[..], &mut sink, 51, &mut buf).unwrap(), 30);

        let data = [0u8; 50];
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 0, &mut buf).unwrap(),
            0
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 1, &mut buf).unwrap(),
            1
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 49, &mut buf).unwrap(),
            49
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 50, &mut buf).unwrap(),
            50
        );
        assert_eq!(copy_n(&mut &data[..], &mut sink, 51, &mut buf).unwrap(), 50);

        let data = [0u8; 80];
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 0, &mut buf).unwrap(),
            0
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 1, &mut buf).unwrap(),
            1
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 49, &mut buf).unwrap(),
            49
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 50, &mut buf).unwrap(),
            50
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 51, &mut buf).unwrap(),
            51
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 79, &mut buf).unwrap(),
            79
        );
        assert_eq!(
            copy_exactly_n(&mut &data[..], &mut sink, 80, &mut buf).unwrap(),
            80
        );
        assert_eq!(copy_n(&mut &data[..], &mut sink, 81, &mut buf).unwrap(), 80);
    }
}
