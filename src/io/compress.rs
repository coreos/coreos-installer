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
use xz2::bufread::XzDecoder;

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
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use CompressDecoder::*;
        match &mut self.decoder {
            Uncompressed(d) => d.read(buf),
            Gzip(d) => {
                let count = d.read(buf)?;
                if count == 0 && !buf.is_empty() {
                    // GzDecoder stops reading as soon as it encounters the
                    // gzip trailer, so it doesn't notice trailing data,
                    // which indicates something wrong with the input.  Try
                    // reading one more byte, and fail if there is one.
                    let mut buf = [0; 1];
                    if d.get_mut().read(&mut buf)? > 0 {
                        return Err(io::Error::new(
                            ErrorKind::InvalidData,
                            "found trailing data after compressed gzip stream",
                        ));
                    }
                }
                Ok(count)
            }
            Xz(d) => d.read(buf),
        }
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
    }
}
