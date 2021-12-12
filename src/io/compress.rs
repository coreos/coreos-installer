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

use anyhow::{Context, Result};
use flate2::bufread::GzDecoder;
use std::io::{self, BufRead, ErrorKind, Read};

use crate::io::XzStreamDecoder;

enum CompressDecoder<R: BufRead> {
    Uncompressed(R),
    Gzip(GzDecoder<R>),
    Xz(XzStreamDecoder<R>),
}

pub struct DecompressReader<R: BufRead> {
    decoder: CompressDecoder<R>,
    allow_trailing: bool,
}

/// Format-sniffing decompressor
impl<R: BufRead> DecompressReader<R> {
    pub fn new(source: R) -> Result<Self> {
        Self::new_full(source, false)
    }

    pub fn for_concatenated(source: R) -> Result<Self> {
        Self::new_full(source, true)
    }

    fn new_full(mut source: R, allow_trailing: bool) -> Result<Self> {
        use CompressDecoder::*;
        let sniff = source.fill_buf().context("sniffing input")?;
        let decoder = if sniff.len() > 2 && &sniff[0..2] == b"\x1f\x8b" {
            Gzip(GzDecoder::new(source))
        } else if sniff.len() > 6 && &sniff[0..6] == b"\xfd7zXZ\x00" {
            Xz(XzStreamDecoder::new(source))
        } else {
            Uncompressed(source)
        };
        Ok(Self {
            decoder,
            allow_trailing,
        })
    }

    pub fn into_inner(self) -> R {
        use CompressDecoder::*;
        match self.decoder {
            Uncompressed(d) => d,
            Gzip(d) => d.into_inner(),
            Xz(d) => d.into_inner(),
        }
    }

    pub fn get_mut(&mut self) -> &mut R {
        use CompressDecoder::*;
        match &mut self.decoder {
            Uncompressed(d) => d,
            Gzip(d) => d.get_mut(),
            Xz(d) => d.get_mut(),
        }
    }

    pub fn compressed(&self) -> bool {
        use CompressDecoder::*;
        match &self.decoder {
            Uncompressed(_) => false,
            Gzip(_) => true,
            Xz(_) => true,
        }
    }
}

impl<R: BufRead> Read for DecompressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use CompressDecoder::*;
        let count = match &mut self.decoder {
            Uncompressed(d) => d.read(buf)?,
            Gzip(d) => d.read(buf)?,
            Xz(d) => d.read(buf)?,
        };
        if count == 0 && !buf.is_empty() && self.compressed() && !self.allow_trailing {
            // Decompressors stop reading as soon as they encounter the
            // compression trailer, so they don't notice trailing data,
            // which indicates something wrong with the input.  Try reading
            // one more byte, and fail if there is one.
            let mut buf = [0; 1];
            if self.get_mut().read(&mut buf)? > 0 {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "found trailing data after compressed stream",
                ));
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    /// Test that DecompressReader fails if data is appended to the
    /// compressed stream.
    #[test]
    fn test_decompress_reader_trailing_data() {
        test_decompress_reader_trailing_data_one(
            &include_bytes!("../../fixtures/verify/1M.gz")[..],
        );
        test_decompress_reader_trailing_data_one(
            &include_bytes!("../../fixtures/verify/1M.xz")[..],
        );
    }

    fn test_decompress_reader_trailing_data_one(input: &[u8]) {
        let mut input = input.to_vec();
        let mut output = Vec::new();

        // successful run
        DecompressReader::new(BufReader::new(&*input))
            .unwrap()
            .read_to_end(&mut output)
            .unwrap();

        // add trailing garbage, make sure we notice
        input.push(0);
        DecompressReader::new(BufReader::new(&*input))
            .unwrap()
            .read_to_end(&mut output)
            .unwrap_err();

        // use concatenated mode, make sure we ignore trailing garbage
        let mut reader = BufReader::new(&*input);
        DecompressReader::for_concatenated(&mut reader)
            .unwrap()
            .read_to_end(&mut output)
            .unwrap();
        let mut remainder = Vec::new();
        reader.read_to_end(&mut remainder).unwrap();
        assert_eq!(&remainder, &[0]);
    }
}
