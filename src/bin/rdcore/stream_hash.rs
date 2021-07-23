// Copyright 2020 CoreOS, Inc.
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

use anyhow::{bail, Context, Result};
use openssl::sha;
use regex::Regex;
use std::fs::OpenOptions;
use std::io::{self, stdin, stdout, BufRead, BufReader, Read, Write};

use crate::cmdline::*;

const MAX_CHUNK_SIZE: usize = 64 * 1024 * 1024;

/// Copy a stream from stdin to stdout, verifying hashes of the data as we
/// go.  Receives an input file in the following format:
///
/// stream-hash sha256 <chunk-size>
/// <hexdigest>
/// <hexdigest>
/// <hexdigest>
/// ...
///
/// Each digest represents exactly <chunk-size> bytes, except for the last
/// one, which represents from one to <chunk-size> bytes.
///
/// We read <chunk-size> bytes into RAM, check their digest, write them to
/// stdout, and repeat.  We never write data to stdout until it's been
/// verified, ensuring that the next program in the shell pipeline never
/// sees untrusted data.
pub fn stream_hash(config: &StreamHashConfig) -> Result<()> {
    let mut hash_file = OpenOptions::new()
        .read(true)
        .open(&config.hash_file)
        .with_context(|| format!("opening {}", config.hash_file))?;
    do_stream_hash(&mut hash_file, &mut stdin(), &mut stdout())
}

fn do_stream_hash(
    hash_file: &mut impl Read,
    input: &mut impl Read,
    output: &mut impl Write,
) -> Result<()> {
    // get buffered reader
    let mut hash_file = BufReader::new(hash_file);

    // read header line
    let mut line = String::new();
    if hash_file
        .read_line(&mut line)
        .context("reading hash file")?
        == 0
    {
        bail!("hash file is empty");
    }

    // parse it
    let captures = Regex::new(r"^stream-hash ([a-z0-9]+) ([0-9]+)\n$")
        .expect("compiling RE")
        .captures(&line)
        .context("couldn't parse hash file header")?;
    let hash_func = match captures
        .get(1)
        .expect("digest algorithm not found")
        .as_str()
    {
        "sha256" => sha::sha256,
        d => bail!("unknown digest algorithm {}", d),
    };
    let chunk_size = captures
        .get(2)
        .expect("chunk size not found")
        .as_str()
        .parse::<usize>()
        .context("couldn't parse chunk size")?;
    if chunk_size == 0 {
        bail!("chunk size cannot be zero");
    } else if chunk_size > MAX_CHUNK_SIZE {
        bail!(
            "chunk size {} is greater than maximum {}",
            chunk_size,
            MAX_CHUNK_SIZE
        );
    }

    // iterate over hashes
    let mut buf = vec![0u8; chunk_size];
    let mut offset: u64 = 0;
    for line in hash_file.lines() {
        // get expected hash
        let line = line.context("couldn't read hash from hash file")?;
        let expected_hash =
            hex::decode(&line).with_context(|| format!("couldn't decode hash: {:?}", line))?;

        // read data
        let mut count = 0;
        loop {
            count += match input.read(&mut buf[count..]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("reading input"),
            };
        }
        if count == 0 {
            bail!("premature end of input data at offset {}", offset);
        }

        // hash and compare
        let data = &buf[..count];
        let found_hash = hash_func(data);
        if expected_hash != found_hash {
            bail!(
                "hash mismatch at offset {}; expected {}, found {}",
                offset,
                hex::encode(expected_hash),
                hex::encode(found_hash)
            );
        }

        // write out buffer
        output.write_all(data).context("writing output")?;
        offset += data.len() as u64;
    }

    // ran out of hashes; make sure we ran out of data
    if input.read(&mut buf[..1]).context("draining input")? != 0 {
        bail!("found extra input data at offset {}", offset);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_stream_hash() {
        struct Test {
            hash_file: &'static str,
            input: &'static str,
            err: Option<&'static str>,
        }
        let tests = vec![
            Test {
                hash_file: "",
                input: "",
                err: Some("hash file is empty"),
            },
            Test {
                hash_file: "aardvark\n",
                input: "",
                err: Some("couldn't parse hash file header"),
            },
            Test {
                hash_file: " stream-hash sha256 1234 \n",
                input: "",
                err: Some("couldn't parse hash file header"),
            },
            Test {
                hash_file: "stream-hash sha255 1234\n",
                input: "",
                err: Some("unknown digest algorithm sha255"),
            },
            Test {
                hash_file: "stream-hash sha256 0\n",
                input: "",
                err: Some("chunk size cannot be zero"),
            },
            Test {
                hash_file: "stream-hash sha256 134217728\n",
                input: "",
                err: Some("chunk size 134217728 is greater than maximum 67108864"),
            },
            Test {
                hash_file: "stream-hash sha256 8\nasdf\n",
                input: "",
                err: Some("couldn't decode hash: \"asdf\""),
            },
            // empty input
            Test {
                hash_file: "stream-hash sha256 8\n",
                input: "",
                err: None,
            },
            // extra hash
            Test {
                hash_file: "stream-hash sha256 8
3af36011654a7bc5159ecf41c610f1f7dbd9deb0d5638f8626db66f7b6467106
3af36011654a7bc5159ecf41c610f1f7dbd9deb0d5638f8626db66f7b6467106
",
                input: "asdfasd\n",
                err: Some("premature end of input data at offset 8"),
            },
            // bad hash
            Test {
                hash_file: "stream-hash sha256 8
3af36011654a7bc5159ecf41c610f1f7dbd9deb0d5638f8626db66f7b6467106
e1bc8d3ba4afc7e109612cb73acbdddac052c93025aa1f82942edabb7deb82a1
",
                input: "asdfasd\nasdf\n",
                err: Some("hash mismatch at offset 8; expected e1bc8d3ba4afc7e109612cb73acbdddac052c93025aa1f82942edabb7deb82a1, found d1bc8d3ba4afc7e109612cb73acbdddac052c93025aa1f82942edabb7deb82a1"),
            },
            // extra data
            Test {
                hash_file: "stream-hash sha256 8
3af36011654a7bc5159ecf41c610f1f7dbd9deb0d5638f8626db66f7b6467106
",
                input: "asdfasd\nqqq",
                err: Some("found extra input data at offset 8"),
            },
            // partial last chunk
            Test {
                hash_file: "stream-hash sha256 8
3af36011654a7bc5159ecf41c610f1f7dbd9deb0d5638f8626db66f7b6467106
5f70ae29b3019ec851ef6b664b59d3fd88dda0de5eb58212ddbd97c65c3f8198
",
                input: "asdfasd\nqwer\n",
                err: None,
            },
            // full last chunk
            Test {
                hash_file: "stream-hash sha256 8
3af36011654a7bc5159ecf41c610f1f7dbd9deb0d5638f8626db66f7b6467106
ef2323b075d71f44c62f62d37b29a5fc4f10c03579a3f6e5b00c2d9666a75e65
",
                input: "asdfasd\nqwertyu\n",
                err: None,
            },
            // no trailing newline in hash file
            Test {
                hash_file: "stream-hash sha256 8
688787d8ff144c502c7f5cffaafe2cc588d86079f9de88304c26b0cb99ce91c6",
                input: "asd",
                err: None,
            },
        ];
        for (i, test) in tests.iter().enumerate() {
            let mut output: Vec<u8> = Vec::new();
            match do_stream_hash(
                &mut Cursor::new(&test.hash_file),
                &mut Cursor::new(&test.input),
                &mut output,
            ) {
                Ok(_) => {
                    assert!(
                        test.err.is_none(),
                        "{}: expected error: {}",
                        i,
                        test.err.unwrap_or("-")
                    );
                    assert_eq!(test.input.as_bytes(), output.as_slice(), "{}", i);
                }
                Err(e) => {
                    assert!(test.err.is_some(), "{}: found error: {}", i, e);
                    assert_eq!(&e.to_string(), test.err.unwrap(), "{}", i);
                }
            }
        }
    }
}
