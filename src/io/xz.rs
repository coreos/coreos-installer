// Copyright 2021 Red Hat
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

// Implementation of an API similar to xz2::bufread::XzDecoder using
// xz2::write::XzDecoder.  We need this because bufread::XzDecoder returns
// io::ErrorKind::InvalidData if there's trailing data after an xz stream
// (which can't be disambiguated from an actual error) but write::XzDecoder
// returns Ok(0).  Return Ok(0) in this case and allow the caller to decide
// what it wants to do about trailing data.
//
// https://github.com/alexcrichton/xz2-rs/pull/86

use bytes::{Buf, BufMut, BytesMut};
use std::io::{self, BufRead, Read};
use xz2::write::XzDecoder;

use crate::io::*;

pub struct XzStreamDecoder<R: BufRead> {
    source: R,
    decompressor: XzDecoder<bytes::buf::Writer<BytesMut>>,
}

impl<R: BufRead> XzStreamDecoder<R> {
    pub fn new(source: R) -> Self {
        Self {
            source,
            decompressor: XzDecoder::new(BytesMut::new().writer()),
        }
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.source
    }

    pub fn into_inner(self) -> R {
        self.source
    }
}

impl<R: BufRead> Read for XzStreamDecoder<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let buf = self.decompressor.get_mut().get_mut();
            if !buf.is_empty() {
                let count = buf.len().min(out.len());
                buf.copy_to_slice(&mut out[..count]);
                return Ok(count);
            }
            let in_ = self.source.fill_buf()?;
            if in_.is_empty() {
                // EOF
                self.decompressor.finish()?;
                return Ok(0);
            }
            let count = self.decompressor.write(in_)?;
            if count == 0 {
                // end of compressed data
                return Ok(0);
            }
            self.source.consume(count);
            // decompressor normally wouldn't fill buf until the next
            // write call
            self.decompressor.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    use xz2::read::XzDecoder;

    #[test]
    fn small_decode() {
        let mut compressed = Vec::new();
        compressed.extend(include_bytes!("../../fixtures/verify/1M.xz"));
        let mut uncompressed = Vec::new();
        XzDecoder::new(&*compressed)
            .read_to_end(&mut uncompressed)
            .unwrap();
        compressed.extend(b"abcdefg");

        let mut d = XzStreamDecoder::new(BufReader::with_capacity(1, &*compressed));
        let mut out = Vec::new();
        let mut buf = [0u8];
        loop {
            match d.read(&mut buf).unwrap() {
                0 => break,
                1 => out.push(buf[0]),
                _ => unreachable!(),
            }
        }
        assert_eq!(&out, &uncompressed);
        let mut remainder = Vec::new();
        d.into_inner().read_to_end(&mut remainder).unwrap();
        assert_eq!(&remainder, b"abcdefg");
    }
}
