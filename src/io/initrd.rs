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
use std::io::{BufRead, BufReader, Cursor, Read};
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
pub fn extract_initrd<R: Read>(source: R, path: &str) -> Result<Option<Vec<u8>>> {
    let mut source = BufReader::with_capacity(BUFFER_SIZE, source);
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
            if entry.name() == path {
                let mut result = Vec::with_capacity(entry.file_size() as usize);
                reader
                    .read_to_end(&mut result)
                    .context("reading CPIO entry contents")?;
                return Ok(Some(result));
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
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xz2::read::XzDecoder;

    #[test]
    fn test_cpio_roundtrip() {
        let input = r#"{}"#;
        let cpio = make_initrd(&[("z", input.as_bytes())]).unwrap();
        let output = extract_initrd(&*cpio, "z").unwrap().unwrap();
        assert_eq!(input.as_bytes(), output.as_slice());
    }

    #[test]
    fn test_cpio_compression() {
        let mut archive: Vec<u8> = Vec::new();
        XzDecoder::new(&include_bytes!("../../fixtures/initrd/compressed.img.xz")[..])
            .read_to_end(&mut archive)
            .unwrap();
        for dir in &["uncompressed-1", "gzip", "xz", "uncompressed-2"] {
            assert_eq!(
                extract_initrd(&*archive, &format!("{}/hello", dir))
                    .unwrap()
                    .unwrap(),
                b"HELLO\n"
            );
            assert_eq!(
                extract_initrd(&*archive, &format!("{}/world", dir))
                    .unwrap()
                    .unwrap(),
                b"WORLD\n"
            );
        }
        assert!(extract_initrd(&*archive, "z").unwrap().is_none());
    }
}
