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

use anyhow::{anyhow, Context, Result};
use cpio::{write_cpio, NewcBuilder, NewcReader};
use lazy_static::lazy_static;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read};
use xz2::stream::{Check, Stream};
use xz2::write::XzEncoder;

use crate::io::*;

lazy_static! {
    static ref ALL_GLOB: GlobMatcher = GlobMatcher::new(&["*"]).unwrap();
}

#[derive(Default, Debug)]
pub struct Initrd {
    members: BTreeMap<String, Vec<u8>>,
}

impl Initrd {
    /// Generate an xz-compressed initrd.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        // kernel requires CRC32: https://www.kernel.org/doc/Documentation/xz.txt
        let mut encoder = XzEncoder::new_stream(
            Vec::new(),
            Stream::new_easy_encoder(9, Check::Crc32).context("creating XZ encoder")?,
        );
        write_cpio(
            self.members.iter().map(|(path, contents)|
            // S_IFREG | 0644
            (NewcBuilder::new(path).mode(0o100_644),
            Cursor::new(contents))),
            &mut encoder,
        )
        .context("writing CPIO archive")?;
        encoder.finish().context("closing XZ compressor")
    }

    /// Read an initrd containing compressed and/or uncompressed archives.
    pub fn from_reader<R: Read>(source: R) -> Result<Self> {
        Self::from_reader_filtered(source, &ALL_GLOB)
    }

    /// Read an initrd containing compressed and/or uncompressed archives,
    /// ignoring paths not matching the specified glob patterns.
    pub fn from_reader_filtered<R: Read>(source: R, filter: &GlobMatcher) -> Result<Self> {
        let mut source = BufReader::with_capacity(BUFFER_SIZE, source);
        let mut result = Self::default();
        // loop until EOF
        while !source
            .fill_buf()
            .context("checking for data in initrd")?
            .is_empty()
        {
            // read one archive
            let mut decompressor = DecompressReader::for_concatenated(source)?;
            loop {
                let mut reader = NewcReader::new(decompressor).context("reading CPIO entry")?;
                let entry = reader.entry();
                if entry.is_trailer() {
                    decompressor = reader.finish().context("finishing reading CPIO trailer")?;
                    break;
                }
                let name = entry.name().to_string();
                if entry.mode() & 0o170_000 == 0o100_000 && filter.matches(&name) {
                    // matching regular file
                    let mut buf = Vec::with_capacity(entry.file_size() as usize);
                    reader
                        .read_to_end(&mut buf)
                        .context("reading CPIO entry contents")?;
                    result.members.insert(name, buf);
                }
                decompressor = reader.finish().context("finishing reading CPIO entry")?;
            }

            // finish decompression, if any, and recover source
            if decompressor.compressed() {
                let mut trailing = Vec::new();
                decompressor
                    .read_to_end(&mut trailing)
                    .context("finishing reading compressed archive")?;
                // padding is okay; data is not
                if trailing.iter().any(|v| *v != 0) {
                    bail!("found trailing garbage inside compressed archive");
                }
            }
            source = decompressor.into_inner();

            // skip any zero padding between archives
            loop {
                let buf = source
                    .fill_buf()
                    .context("checking for padding in initrd")?;
                if buf.is_empty() {
                    // EOF
                    break;
                }
                match buf.iter().position(|v| *v != 0) {
                    Some(pos) => {
                        source.consume(pos);
                        break;
                    }
                    None => {
                        let len = buf.len();
                        source.consume(len);
                    }
                }
            }
        }
        Ok(result)
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.members.get(path).map(|v| v.as_slice())
    }

    pub fn find(&self, filter: &GlobMatcher) -> BTreeMap<&str, &[u8]> {
        self.members
            .iter()
            .filter(|(p, _)| filter.matches(p))
            .map(|(p, c)| (p.as_str(), c.as_slice()))
            .collect()
    }

    pub fn add(&mut self, path: &str, contents: Vec<u8>) {
        self.members.insert(path.into(), contents);
    }

    pub fn remove(&mut self, path: &str) {
        self.members.remove(path);
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
}

pub struct GlobMatcher {
    patterns: Vec<glob::Pattern>,
}

impl GlobMatcher {
    pub fn new(globs: &[&str]) -> Result<Self> {
        Ok(Self {
            patterns: globs
                .iter()
                .map(|p| glob::Pattern::new(*p).map_err(|e| anyhow!(e)))
                .collect::<Result<_>>()?,
        })
    }

    fn matches(&self, path: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maplit::btreemap;
    use xz2::read::XzDecoder;

    #[test]
    fn roundtrip() {
        let input = r#"{}"#;
        let mut initrd = Initrd::default();
        initrd.add("z", input.as_bytes().into());
        assert_eq!(
            input.as_bytes(),
            Initrd::from_reader(&*initrd.to_bytes().unwrap())
                .unwrap()
                .get("z")
                .unwrap()
        );
    }

    #[test]
    fn compression() {
        let mut archive: Vec<u8> = Vec::new();
        XzDecoder::new(&include_bytes!("../../fixtures/initrd/compressed.img.xz")[..])
            .read_to_end(&mut archive)
            .unwrap();
        let initrd = Initrd::from_reader(&*archive).unwrap();
        assert_eq!(
            initrd.members,
            btreemap! {
                "uncompressed-1/hello".into() => b"HELLO\n".to_vec(),
                "uncompressed-1/world".into() => b"WORLD\n".to_vec(),
                "uncompressed-2/hello".into() => b"HELLO\n".to_vec(),
                "uncompressed-2/world".into() => b"WORLD\n".to_vec(),
                "gzip/hello".into() => b"HELLO\n".to_vec(),
                "gzip/world".into() => b"WORLD\n".to_vec(),
                "xz/hello".into() => b"HELLO\n".to_vec(),
                "xz/world".into() => b"WORLD\n".to_vec(),
            }
        );
    }

    /// Check that the last copy of a file in an archive wins, which is
    /// how the kernel behaves.
    #[test]
    fn redundancy() {
        let mut archive: Vec<u8> = Vec::new();
        XzDecoder::new(&include_bytes!("../../fixtures/initrd/redundant.img.xz")[..])
            .read_to_end(&mut archive)
            .unwrap();
        assert_eq!(
            Initrd::from_reader(&*archive)
                .unwrap()
                .get("data/file")
                .unwrap(),
            b"third\n"
        );
    }

    #[test]
    fn matching() {
        let mut archive: Vec<u8> = Vec::new();
        XzDecoder::new(&include_bytes!("../../fixtures/initrd/compressed.img.xz")[..])
            .read_to_end(&mut archive)
            .unwrap();

        let matcher = |glob| GlobMatcher::new(&[glob]).unwrap();

        // unfiltered initrd
        let initrd = Initrd::from_reader(&*archive).unwrap();
        assert_eq!(initrd.find(&matcher("gzip/hello")).len(), 1);
        assert_eq!(initrd.find(&matcher("gzip/*")).len(), 2);
        assert_eq!(initrd.find(&matcher("*/hello")).len(), 4);
        assert_eq!(initrd.find(&matcher("*")).len(), 8);
        assert_eq!(initrd.find(&matcher("z")).len(), 0);

        // filtered initrd
        let initrd = Initrd::from_reader_filtered(&*archive, &matcher("z")).unwrap();
        assert_eq!(initrd.find(&matcher("*")).len(), 0);
        let initrd = Initrd::from_reader_filtered(&*archive, &matcher("gzip/*")).unwrap();
        assert_eq!(initrd.find(&matcher("*")).len(), 2);
        let initrd = Initrd::from_reader_filtered(&*archive, &matcher("uncompressed-*")).unwrap();
        assert_eq!(initrd.find(&matcher("*")).len(), 4);
    }
}
