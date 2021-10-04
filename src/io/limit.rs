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

use std::io::{self, Read, Write};

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
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
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

pub struct LimitWriter<W: Write> {
    sink: W,
    length: u64,
    remaining: u64,
    conflict: String,
}

impl<W: Write> LimitWriter<W> {
    pub fn new(sink: W, length: u64, conflict: String) -> Self {
        Self {
            sink,
            length,
            remaining: length,
            conflict,
        }
    }
}

impl<W: Write> Write for LimitWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let allowed = self.remaining.min(buf.len() as u64);
        if allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("collision with {} at offset {}", self.conflict, self.length),
            ));
        }
        let count = self.sink.write(&buf[..allowed as usize])?;
        self.remaining = self
            .remaining
            .checked_sub(count as u64)
            .expect("wrote more bytes than allowed");
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn limit_reader_test() {
        // build input data
        let data: Vec<u8> = (0..100).collect();

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

    #[test]
    fn limit_writer_test() {
        let data: Vec<u8> = (0..100).collect();

        // limit larger than data
        let mut outbuf: Vec<u8> = Vec::new();
        let mut lim = LimitWriter::new(&mut outbuf, 150, "foo".into());
        lim.write_all(&data).unwrap();
        lim.flush().unwrap();
        assert_eq!(data, outbuf);

        // limit exactly equal to data
        let mut outbuf: Vec<u8> = Vec::new();
        let mut lim = LimitWriter::new(&mut outbuf, 100, "foo".into());
        lim.write_all(&data).unwrap();
        lim.flush().unwrap();
        assert_eq!(data, outbuf);

        // limit smaller than data
        let mut outbuf: Vec<u8> = Vec::new();
        let mut lim = LimitWriter::new(&mut outbuf, 90, "foo".into());
        assert_eq!(
            lim.write_all(&data).unwrap_err().to_string(),
            "collision with foo at offset 90"
        );

        // directly test writing in multiple chunks
        let mut outbuf: Vec<u8> = Vec::new();
        let mut lim = LimitWriter::new(&mut outbuf, 90, "foo".into());
        assert_eq!(lim.write(&data[0..60]).unwrap(), 60);
        assert_eq!(lim.write(&data[60..100]).unwrap(), 30); // short write
        assert_eq!(
            lim.write(&data[90..100]).unwrap_err().to_string(),
            "collision with foo at offset 90"
        );
        assert_eq!(lim.write(&data[0..0]).unwrap(), 0);
        assert_eq!(&data[0..90], &outbuf);
    }
}
