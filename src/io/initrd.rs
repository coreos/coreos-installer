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
use cpio::{write_cpio, NewcBuilder, NewcReader};
use std::io::{BufReader, Cursor, Read};
use xz2::stream::{Check, Stream};
use xz2::write::XzEncoder;

use crate::io::*;

/// Make an xz-compressed initrd containing the specified members.
pub fn make_initrd(members: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    // kernel requires CRC32: https://www.kernel.org/doc/Documentation/xz.txt
    let mut encoder = XzEncoder::new_stream(
        Vec::new(),
        Stream::new_easy_encoder(9, Check::Crc32).context("creating XZ encoder")?,
    );
    write_cpio(
        members.iter().map(|(path, contents)|
        // S_IFREG | 0644
        (NewcBuilder::new(path).mode(0o100_644),
        Cursor::new(*contents))),
        &mut encoder,
    )
    .context("writing CPIO archive")?;
    encoder.finish().context("closing XZ compressor")
}

/// Extract a compressed or uncompressed CPIO archive and return the
/// contents of the specified path.
pub fn extract_initrd(buf: &[u8], path: &str) -> Result<Option<Vec<u8>>> {
    // older versions of this program, and its predecessor, compressed
    // with gzip
    let mut decompressor = DecompressReader::new(BufReader::new(buf))?;
    loop {
        let mut reader = NewcReader::new(decompressor).context("reading CPIO entry")?;
        let entry = reader.entry();
        if entry.is_trailer() {
            return Ok(None);
        }
        if entry.name() == path {
            let mut result = Vec::with_capacity(entry.file_size() as usize);
            reader
                .read_to_end(&mut result)
                .context("reading CPIO entry contents")?;
            return Ok(Some(result));
        }
        decompressor = reader.finish().context("finishing reading CPIO entry")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpio_roundtrip() {
        let input = r#"{}"#;
        let cpio = make_initrd(&[("z", input.as_bytes())]).unwrap();
        let output = extract_initrd(&cpio, "z").unwrap().unwrap();
        assert_eq!(input.as_bytes(), output.as_slice());
    }
}
