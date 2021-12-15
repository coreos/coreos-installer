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

//! Wrappers for splitting I/O streams

use std::io::{self, Read, Write};

/// Reader wrapper that copies data to a writer as a side effect
pub struct TeeReader<R: Read, W: Write> {
    source: R,
    dest: W,
}

impl<R: Read, W: Write> TeeReader<R, W> {
    pub fn new(source: R, dest: W) -> Self {
        Self { source, dest }
    }

    pub fn into_inner(self) -> (R, W) {
        (self.source, self.dest)
    }
}

impl<R: Read, W: Write> Read for TeeReader<R, W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let count = self.source.read(buf)?;
        self.dest.write_all(&buf[..count])?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Do some I/O of different sizes, reach EOF, and check that both
    /// copies of the output are correct
    #[test]
    fn tee_reader() {
        const COUNT: usize = 100;
        let src: Vec<u8> = (0..COUNT as u8).collect();
        let mut buf = vec![0; 2 * COUNT];
        let mut off = 0;
        let mut tee = TeeReader::new(&*src, Vec::new());
        for i in 2.. {
            off += tee.read(&mut buf[off..off + i]).unwrap();
            assert!(off <= COUNT);
            if off == COUNT {
                break;
            }
        }
        assert_eq!(src, buf[..COUNT]);
        let (_, dest) = tee.into_inner();
        assert_eq!(src, dest);
    }
}
