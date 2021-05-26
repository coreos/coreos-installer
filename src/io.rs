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

use anyhow::{bail, ensure, Context, Result};
use flate2::read::GzDecoder;
use openssl::sha;
use std::io::{self, BufRead, ErrorKind, Read, Write};
use std::result;
use xz2::read::XzDecoder;

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
            #[allow(clippy::redundant_slicing)]
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

/// Ignition-style message digests
#[derive(Debug)]
pub enum IgnitionHash {
    /// SHA-256 digest.
    Sha256(Vec<u8>),
    /// SHA-512 digest.
    Sha512(Vec<u8>),
}

/// Digest implementation.  Helpfully, each digest in openssl::sha has a
/// different type.
enum IgnitionHasher {
    Sha256(sha::Sha256),
    Sha512(sha::Sha512),
}

impl IgnitionHash {
    /// Try to parse an hash-digest argument.
    ///
    /// This expects an input value following the `ignition.config.verification.hash`
    /// spec, i.e. `<type>-<value>` format.
    pub fn try_parse(input: &str) -> Result<Self> {
        let parts: Vec<_> = input.splitn(2, '-').collect();
        if parts.len() != 2 {
            bail!("failed to detect hash-type and digest in '{}'", input);
        }
        let (hash_kind, hex_digest) = (parts[0], parts[1]);

        let hash = match hash_kind {
            "sha256" => {
                let digest = hex::decode(hex_digest).context("decoding hex digest")?;
                ensure!(
                    digest.len().saturating_mul(8) == 256,
                    "wrong digest length ({})",
                    digest.len().saturating_mul(8)
                );
                IgnitionHash::Sha256(digest)
            }
            "sha512" => {
                let digest = hex::decode(hex_digest).context("decoding hex digest")?;
                ensure!(
                    digest.len().saturating_mul(8) == 512,
                    "wrong digest length ({})",
                    digest.len().saturating_mul(8)
                );
                IgnitionHash::Sha512(digest)
            }
            x => bail!("unknown hash type '{}'", x),
        };

        Ok(hash)
    }

    /// Digest and validate input data.
    pub fn validate(&self, input: &mut impl Read) -> Result<()> {
        let (mut hasher, digest) = match self {
            IgnitionHash::Sha256(val) => (IgnitionHasher::Sha256(sha::Sha256::new()), val),
            IgnitionHash::Sha512(val) => (IgnitionHasher::Sha512(sha::Sha512::new()), val),
        };
        let mut buf = [0u8; 128 * 1024];
        loop {
            match input.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => match hasher {
                    IgnitionHasher::Sha256(ref mut h) => h.update(&buf[..n]),
                    IgnitionHasher::Sha512(ref mut h) => h.update(&buf[..n]),
                },
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("reading input"),
            };
        }
        let computed = match hasher {
            IgnitionHasher::Sha256(h) => h.finish().to_vec(),
            IgnitionHasher::Sha512(h) => h.finish().to_vec(),
        };

        if &computed != digest {
            bail!(
                "hash mismatch, computed '{}' but expected '{}'",
                hex::encode(computed),
                hex::encode(digest),
            );
        }

        Ok(())
    }
}

enum CompressDecoder<R: BufRead> {
    Uncompressed(R),
    Gzip(GzDecoder<R>),
    Xz(XzDecoder<R>),
}

pub struct DecompressReader<R: BufRead> {
    decoder: CompressDecoder<R>,
}

/// Format-sniffing decompressor
impl<R: BufRead> DecompressReader<R> {
    pub fn new(mut source: R) -> Result<Self> {
        use CompressDecoder::*;
        let sniff = source.fill_buf().context("sniffing input")?;
        let decoder = if sniff.len() > 2 && &sniff[0..2] == b"\x1f\x8b" {
            Gzip(GzDecoder::new(source))
        } else if sniff.len() > 6 && &sniff[0..6] == b"\xfd7zXZ\x00" {
            Xz(XzDecoder::new(source))
        } else {
            Uncompressed(source)
        };
        Ok(Self { decoder })
    }
}

impl<R: BufRead> Read for DecompressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> result::Result<usize, io::Error> {
        use CompressDecoder::*;
        match &mut self.decoder {
            Uncompressed(d) => d.read(buf),
            Gzip(d) => d.read(buf),
            Xz(d) => d.read(buf),
        }
    }
}

pub struct LimitReader<R: Read> {
    source: R,
    length: u64,
    remaining: u64,
    conflict: String,
}

impl<R: Read> LimitReader<R> {
    pub fn new(source: R, length: u64, conflict: String) -> Self {
        Self {
            source,
            length,
            remaining: length,
            conflict,
        }
    }
}

impl<R: Read> Read for LimitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> result::Result<usize, io::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let allowed = self.remaining.min(buf.len() as u64);
        if allowed == 0 {
            // reached the limit; only error if we're not at EOF
            return match self.source.read(&mut buf[..1]) {
                Ok(0) => Ok(0),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("collision with {} at offset {}", self.conflict, self.length),
                )),
                Err(e) => Err(e),
            };
        }
        let count = self.source.read(&mut buf[..allowed as usize])?;
        self.remaining = self
            .remaining
            .checked_sub(count as u64)
            .expect("read more bytes than allowed");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_ignition_hash_cli_parse() {
        let err_cases = vec!["", "foo-bar", "-bar", "sha512", "sha512-", "sha512-00"];
        for arg in err_cases {
            IgnitionHash::try_parse(arg).expect_err(&format!("input: {}", arg));
        }

        let null_digest = "sha512-cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e";
        IgnitionHash::try_parse(null_digest).unwrap();
    }

    #[test]
    fn test_ignition_hash_validate() {
        let input = vec![b'a', b'b', b'c'];
        let hash_args = [
            (true, "sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            (true, "sha512-ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"),
            (false, "sha256-aa7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            (false, "sha512-cdaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f")
        ];
        for (valid, hash_arg) in &hash_args {
            let hasher = IgnitionHash::try_parse(&hash_arg).unwrap();
            let mut rd = std::io::Cursor::new(&input);
            assert!(hasher.validate(&mut rd).is_ok() == *valid);
        }
    }

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

    #[test]
    fn limit_reader_test() {
        // build input data
        let mut data: Vec<u8> = Vec::new();
        for i in 0..100 {
            data.push(i);
        }

        // limit larger than file
        let mut file = Cursor::new(data.clone());
        let mut lim = LimitReader::new(&mut file, 150, "foo".into());
        let mut buf = [0u8; 60];
        assert_eq!(lim.read(&mut buf).unwrap(), 60);
        assert_eq!(buf[..], data[0..60]);
        assert_eq!(lim.read(&mut buf).unwrap(), 40);
        assert_eq!(buf[..40], data[60..100]);
        assert_eq!(lim.read(&mut buf).unwrap(), 0);

        // limit exactly equal to file
        let mut file = Cursor::new(data.clone());
        let mut lim = LimitReader::new(&mut file, 100, "foo".into());
        let mut buf = [0u8; 60];
        assert_eq!(lim.read(&mut buf).unwrap(), 60);
        assert_eq!(buf[..], data[0..60]);
        assert_eq!(lim.read(&mut buf).unwrap(), 40);
        assert_eq!(buf[..40], data[60..100]);
        assert_eq!(lim.read(&mut buf).unwrap(), 0);

        // buffer smaller than limit
        let mut file = Cursor::new(data.clone());
        let mut lim = LimitReader::new(&mut file, 90, "foo".into());
        let mut buf = [0u8; 60];
        assert_eq!(lim.read(&mut buf).unwrap(), 60);
        assert_eq!(buf[..], data[0..60]);
        assert_eq!(lim.read(&mut buf).unwrap(), 30);
        assert_eq!(buf[..30], data[60..90]);
        assert_eq!(
            lim.read(&mut buf).unwrap_err().to_string(),
            "collision with foo at offset 90"
        );

        // buffer exactly equal to limit
        let mut file = Cursor::new(data.clone());
        let mut lim = LimitReader::new(&mut file, 60, "foo".into());
        let mut buf = [0u8; 60];
        assert_eq!(lim.read(&mut buf).unwrap(), 60);
        assert_eq!(buf[..], data[0..60]);
        assert_eq!(
            lim.read(&mut buf).unwrap_err().to_string(),
            "collision with foo at offset 60"
        );

        // buffer larger than limit
        let mut file = Cursor::new(data.clone());
        let mut lim = LimitReader::new(&mut file, 50, "foo".into());
        let mut buf = [0u8; 60];
        assert_eq!(lim.read(&mut buf).unwrap(), 50);
        assert_eq!(buf[..50], data[0..50]);
        assert_eq!(
            lim.read(&mut buf).unwrap_err().to_string(),
            "collision with foo at offset 50"
        );
    }
}
