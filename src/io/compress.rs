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
use std::io::{self, ErrorKind, Read};

use crate::io::{is_zstd_magic, PeekReader, XzStreamDecoder, ZstdStreamDecoder};

enum CompressDecoder<'a, R: Read> {
    Uncompressed(PeekReader<R>),
    Gzip(GzDecoder<PeekReader<R>>),
    Xz(XzStreamDecoder<PeekReader<R>>),
    Zstd(ZstdStreamDecoder<'a, R>),
}

pub struct DecompressReader<'a, R: Read> {
    decoder: CompressDecoder<'a, R>,
    allow_trailing: bool,
}

/// Format-sniffing decompressor
impl<R: Read> DecompressReader<'_, R> {
    pub fn new(source: PeekReader<R>) -> Result<Self> {
        Self::new_full(source, false)
    }

    pub fn for_concatenated(source: PeekReader<R>) -> Result<Self> {
        Self::new_full(source, true)
    }

    fn new_full(mut source: PeekReader<R>, allow_trailing: bool) -> Result<Self> {
        use CompressDecoder::*;
        let sniff = source.peek(6).context("sniffing input")?;
        let decoder = if sniff.len() >= 2 && &sniff[0..2] == b"\x1f\x8b" {
            Gzip(GzDecoder::new(source))
        } else if sniff.len() >= 6 && &sniff[0..6] == b"\xfd7zXZ\x00" {
            Xz(XzStreamDecoder::new(source))
        } else if sniff.len() > 4 && is_zstd_magic(sniff[0..4].try_into().unwrap()) {
            Zstd(ZstdStreamDecoder::new(source)?)
        } else {
            Uncompressed(source)
        };
        Ok(Self {
            decoder,
            allow_trailing,
        })
    }

    pub fn into_inner(self) -> PeekReader<R> {
        use CompressDecoder::*;
        match self.decoder {
            Uncompressed(d) => d,
            Gzip(d) => d.into_inner(),
            Xz(d) => d.into_inner(),
            Zstd(d) => d.into_inner(),
        }
    }

    pub fn get_mut(&mut self) -> &mut PeekReader<R> {
        use CompressDecoder::*;
        match &mut self.decoder {
            Uncompressed(d) => d,
            Gzip(d) => d.get_mut(),
            Xz(d) => d.get_mut(),
            Zstd(d) => d.get_mut(),
        }
    }

    pub fn compressed(&self) -> bool {
        use CompressDecoder::*;
        match &self.decoder {
            Uncompressed(_) => false,
            Gzip(_) => true,
            Xz(_) => true,
            Zstd(_) => true,
        }
    }
}

impl<R: Read> Read for DecompressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use CompressDecoder::*;
        let count = match &mut self.decoder {
            Uncompressed(d) => d.read(buf)?,
            Gzip(d) => d.read(buf)?,
            Xz(d) => d.read(buf)?,
            Zstd(d) => d.read(buf)?,
        };
        if count == 0 && !buf.is_empty() && self.compressed() && !self.allow_trailing {
            // Decompressors stop reading as soon as they encounter the
            // compression trailer, so they don't notice trailing data,
            // which indicates something wrong with the input.  Look for
            // one more byte, and fail if there is one.
            if !self.get_mut().peek(1)?.is_empty() {
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
        test_decompress_reader_trailing_data_one(
            &include_bytes!("../../fixtures/verify/1M.zst")[..],
        );
    }

    fn test_decompress_reader_trailing_data_one(input: &[u8]) {
        let mut input = input.to_vec();
        let mut output = Vec::new();

        // successful run
        DecompressReader::new(PeekReader::with_capacity(32, &*input))
            .unwrap()
            .read_to_end(&mut output)
            .unwrap();

        // drop last byte, make sure we notice
        DecompressReader::new(PeekReader::with_capacity(32, &input[0..input.len() - 1]))
            .unwrap()
            .read_to_end(&mut output)
            .unwrap_err();

        // add trailing garbage, make sure we notice
        input.push(0);
        DecompressReader::new(PeekReader::with_capacity(32, &*input))
            .unwrap()
            .read_to_end(&mut output)
            .unwrap_err();

        // use concatenated mode, make sure we ignore trailing garbage
        let mut reader =
            DecompressReader::for_concatenated(PeekReader::with_capacity(32, &*input)).unwrap();
        reader.read_to_end(&mut output).unwrap();
        let mut remainder = Vec::new();
        reader.into_inner().read_to_end(&mut remainder).unwrap();
        assert_eq!(&remainder, &[0]);
    }
}
