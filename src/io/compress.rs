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
use std::io::{self, BufRead, Read};
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
            Gzip(d) => d.read(buf),
            Xz(d) => d.read(buf),
        }
    }
}
