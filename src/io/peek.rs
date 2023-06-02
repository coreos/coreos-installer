// Copyright 2022 Red Hat, Inc.
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

// Read wrapper that allows peeking ahead in the stream without consuming
// the peeked bytes.  BufRead.fill_buf() does not provide this, since it
// only guarantees to return one byte.  For simplicity, we implement this
// as a thin wrapper around BufReader.

use bytes::{Buf, BytesMut};
use std::io::{BufRead, BufReader, Read, Result, Seek, SeekFrom};

pub struct PeekReader<R: Read> {
    source: BufReader<R>,
    buf: BytesMut,
}

impl<R: Read> PeekReader<R> {
    pub fn with_capacity(capacity: usize, inner: R) -> Self {
        Self {
            source: BufReader::with_capacity(capacity, inner),
            buf: BytesMut::new(),
        }
    }

    /// Return the next amt bytes without consuming them.  May return fewer
    /// bytes at EOF.
    pub fn peek(&mut self, amt: usize) -> Result<&[u8]> {
        if self.buf.remaining() < amt {
            let mut extend = amt - self.buf.remaining();
            self.buf.resize(amt, 0);
            while extend > 0 {
                let start = self.buf.len() - extend;
                let count = self.source.read(&mut self.buf[start..])?;
                if count == 0 {
                    // EOF
                    self.buf.truncate(start);
                    break;
                }
                extend -= count;
            }
        }
        Ok(&self.buf[..self.buf.len().min(amt)])
    }

    // no direct access to inner source, since that would lose data if
    // buf is non-empty
}

impl<R: Read> Read for PeekReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.buf.has_remaining() {
            let count = buf.len().min(self.buf.remaining());
            self.buf.copy_to_slice(&mut buf[..count]);
            return Ok(count);
        }
        self.source.read(buf)
    }
}

impl<R: Read + Seek> Seek for PeekReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.buf.clear();
        self.source.seek(pos)
    }
}

impl<R: Read> BufRead for PeekReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.buf.has_remaining() {
            Ok(&self.buf)
        } else {
            self.source.fill_buf()
        }
    }

    fn consume(&mut self, amt: usize) {
        if self.buf.has_remaining() {
            assert!(amt <= self.buf.remaining());
            self.buf.advance(amt);
        } else {
            self.source.consume(amt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_peek() -> PeekReader<Cursor<&'static [u8]>> {
        // use BufReader capacity larger than input; we're not testing
        // BufReader's buffering behavior
        PeekReader::with_capacity(64, Cursor::new(b"abcdefghijklmnopqrstuvwxyz"))
    }

    fn read_bytes<R: Read>(peek: &mut PeekReader<R>, amt: usize) -> Vec<u8> {
        let mut buf = vec![0; amt];
        let amt = peek.read(&mut buf).unwrap();
        buf.truncate(amt);
        buf
    }

    #[test]
    fn read() {
        let mut peek = make_peek();
        // read some bytes
        assert_eq!(&read_bytes(&mut peek, 3), b"abc");
        assert_eq!(&read_bytes(&mut peek, 3), b"def");
        // peek at some bytes
        assert_eq!(peek.peek(2).unwrap(), b"gh");
        // peek reuses existing buffer
        assert_eq!(peek.peek(1).unwrap(), b"g");
        // peek extends buffer
        assert_eq!(peek.peek(4).unwrap(), b"ghij");
        // read after peek, partially emptying buffer
        assert_eq!(&read_bytes(&mut peek, 3), b"ghi");
        // peek extends buffer
        assert_eq!(peek.peek(2).unwrap(), b"jk");
        // read after peek, emptying buffer
        assert_eq!(&read_bytes(&mut peek, 3), b"jk");
        // normal read
        assert_eq!(&read_bytes(&mut peek, 3), b"lmn");
    }

    #[test]
    fn seek() {
        let mut peek = make_peek();
        // fill peek buffer
        assert_eq!(peek.peek(4).unwrap(), b"abcd");
        // seek
        peek.seek(SeekFrom::Start(10)).unwrap();
        // read
        assert_eq!(&read_bytes(&mut peek, 3), b"klm");
        // fill peek buffer
        assert_eq!(peek.peek(4).unwrap(), b"nopq");
        // seek
        peek.seek(SeekFrom::Start(5)).unwrap();
        // peek
        assert_eq!(peek.peek(4).unwrap(), b"fghi");
    }

    #[test]
    fn buf() {
        let mut peek = make_peek();
        // BufRead fill and partial consume
        assert_eq!(peek.fill_buf().unwrap(), b"abcdefghijklmnopqrstuvwxyz");
        peek.consume(5);
        // BufRead fill
        assert_eq!(peek.fill_buf().unwrap(), b"fghijklmnopqrstuvwxyz");
        // peek
        assert_eq!(peek.peek(5).unwrap(), b"fghij");
        // Peek buffer fill and partial consume
        assert_eq!(peek.fill_buf().unwrap(), b"fghij");
        peek.consume(3);
        // Peek buffer fill and consume
        assert_eq!(peek.fill_buf().unwrap(), b"ij");
        peek.consume(2);
        // BufRead fill
        assert_eq!(peek.fill_buf().unwrap(), b"klmnopqrstuvwxyz");
    }

    #[test]
    fn eof() {
        let mut peek = make_peek();
        // seek to near end
        peek.seek(SeekFrom::Start(24)).unwrap();
        // peek past end
        assert_eq!(peek.peek(4).unwrap(), b"yz");
        // read to end
        assert_eq!(&read_bytes(&mut peek, 3), b"yz");
        // peek at end
        assert_eq!(peek.peek(4).unwrap(), b"");
    }
}
