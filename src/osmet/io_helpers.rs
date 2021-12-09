// Copyright 2020 Red Hat, Inc.
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

use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::io::Sha256Digest;

// ab/cdef....file --> 0xabcdef...
pub fn object_path_to_checksum(path: &Path) -> Result<Sha256Digest> {
    let chksum2 = path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let chksum62 = path
        .file_stem()
        .unwrap()
        .to_str()
        .with_context(|| format!("invalid non-UTF-8 object filename: {:?}", path))?;
    if chksum2.len() != 2 || chksum62.len() != 62 {
        bail!("Malformed object path {:?}", path);
    }

    let mut bin_chksum = [0u8; 32];
    bin_chksum[0] = u8::from_str_radix(chksum2, 16)?;
    for i in 0..31 {
        bin_chksum[i + 1] = u8::from_str_radix(&chksum62[i * 2..(i + 1) * 2], 16)?;
    }

    Ok(Sha256Digest(bin_chksum))
}

// 0xabcdef... --> ab/cdef....file
pub fn checksum_to_object_path(chksum: &Sha256Digest, buf: &mut Vec<u8>) -> Result<()> {
    write!(buf, "{:02x}/", chksum.0[0])?;
    for i in 1..32 {
        write!(buf, "{:02x}", chksum.0[i])?;
    }
    write!(buf, ".file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn test_checksum_to_object_path() {
        let mut chksum = Sha256Digest([0u8; 32]);
        let mut buf: Vec<u8> = Vec::new();

        // all zeros
        checksum_to_object_path(&chksum, &mut buf).unwrap();
        assert_eq!(
            Path::new(OsStr::from_bytes(buf.as_slice())),
            Path::new("00/00000000000000000000000000000000000000000000000000000000000000.file")
        );
        buf.truncate(0);

        // not all zeros
        chksum.0[0] = 0xff;
        chksum.0[1] = 0xfe;
        chksum.0[31] = 0xfd;
        checksum_to_object_path(&chksum, &mut buf).unwrap();
        assert_eq!(
            Path::new(OsStr::from_bytes(buf.as_slice())),
            Path::new("ff/fe0000000000000000000000000000000000000000000000000000000000fd.file")
        );
        buf.truncate(0);
    }
}
