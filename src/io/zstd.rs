// Copyright 2022 Red Hat
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

// Implementation of an API similar to zstd::stream::read::Decoder using
// zstd::stream::raw::Decoder.  We need this because read::Decoder returns
// io::ErrorKind::Other if there's trailing data after a zstd stream, which
// can't be disambiguated from an actual error.  By using the low-level API,
// we can check zstd::stream::raw::Status.remaining to see whether the
// decoder thinks it's at the end of a frame, check the upcoming bytes for
// the magic number of another frame, and decide whether we're done.  The
// raw decoder always stops at frame boundaries, so this is reliable.  If
// done, return Ok(0) and allow the caller to decide what it wants to do
// about trailing data.

use anyhow::{Context, Result};
use bytes::{Buf, BytesMut};
use std::io::{self, BufRead, Error, ErrorKind, Read};
use zstd::stream::raw::{Decoder, Operation};
use zstd::zstd_safe::{MAGICNUMBER, MAGIC_SKIPPABLE_MASK, MAGIC_SKIPPABLE_START};

use crate::io::PeekReader;

pub struct ZstdStreamDecoder<'a, R: Read> {
    source: PeekReader<R>,
    buf: BytesMut,
    decoder: Decoder<'a>,
    start_of_frame: bool,
}

impl<R: Read> ZstdStreamDecoder<'_, R> {
    pub fn new(source: PeekReader<R>) -> Result<Self> {
        Ok(Self {
            source,
            buf: BytesMut::new(),
            decoder: Decoder::new().context("creating zstd decoder")?,
            start_of_frame: true,
        })
    }

    pub fn get_mut(&mut self) -> &mut PeekReader<R> {
        &mut self.source
    }

    pub fn into_inner(self) -> PeekReader<R> {
        self.source
    }
}

impl<R: Read> Read for ZstdStreamDecoder<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if !self.buf.is_empty() {
                let count = self.buf.len().min(out.len());
                self.buf.copy_to_slice(&mut out[..count]);
                return Ok(count);
            }
            if self.start_of_frame {
                let peek = self.source.peek(4)?;
                if peek.len() < 4 || !is_zstd_magic(peek[0..4].try_into().unwrap()) {
                    // end of compressed data
                    return Ok(0);
                }
                self.start_of_frame = false;
            }
            let in_ = self.source.fill_buf()?;
            if in_.is_empty() {
                return Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "premature EOF reading zstd frame",
                ));
            }
            // unfortunately we have to initialize to 0 for safety
            // BUFFER_SIZE is very large; use a smaller buffer to avoid
            // unneeded reinitialization
            self.buf.resize(16384, 0);
            let status = self.decoder.run_on_buffers(in_, &mut self.buf)?;
            self.source.consume(status.bytes_read);
            self.buf.truncate(status.bytes_written);
            if status.remaining == 0 {
                self.start_of_frame = true;
            }
        }
    }
}

pub fn is_zstd_magic(buf: [u8; 4]) -> bool {
    let val = u32::from_le_bytes(buf);
    val == MAGICNUMBER || val & MAGIC_SKIPPABLE_MASK == MAGIC_SKIPPABLE_START
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_decode() {
        let mut compressed = Vec::new();
        compressed.extend(include_bytes!("../../fixtures/verify/1M.zst"));
        let uncompressed = zstd::stream::decode_all(&*compressed).unwrap();
        compressed.extend(b"abcdefg");

        let mut d = ZstdStreamDecoder::new(PeekReader::with_capacity(1, &*compressed)).unwrap();
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
