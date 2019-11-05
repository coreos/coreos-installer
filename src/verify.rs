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

use error_chain::bail;
use std::fs::{metadata, set_permissions, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::result;
use tempdir::TempDir;

use crate::errors::*;

pub struct GpgReader<R: Read> {
    _gpgdir: TempDir,
    source: R,
    child: Child,
}

impl<R: Read> GpgReader<R> {
    pub fn new(source: R, signature: &[u8]) -> Result<Self> {
        // create GPG home directory with restrictive mode
        let gpgdir =
            TempDir::new("coreos-installer").chain_err(|| "creating temporary directory")?;
        let meta =
            metadata(gpgdir.path()).chain_err(|| "getting metadata for temporary directory")?;
        let mut permissions = meta.permissions();
        permissions.set_mode(0o700);
        set_permissions(gpgdir.path(), permissions)
            .chain_err(|| "setting mode for temporary directory")?;

        // import public keys
        let keys = include_bytes!("signing-keys.asc");
        let mut import = Command::new("gpg")
            .arg("--homedir")
            .arg(gpgdir.path())
            .arg("--batch")
            .arg("--quiet")
            .arg("--import")
            .stdin(Stdio::piped())
            .spawn()
            .chain_err(|| "running gpg --import")?;
        import
            .stdin
            .as_mut()
            .unwrap()
            .write_all(keys)
            .chain_err(|| "importing GPG keys")?;
        if !import
            .wait()
            .chain_err(|| "waiting for gpg --import")?
            .success()
        {
            bail!("gpg --import failed");
        }

        // write signature to file
        let mut signature_path = gpgdir.path().to_path_buf();
        signature_path.push("signature");
        let mut signature_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&signature_path)
            .chain_err(|| "creating signature file")?;
        signature_file
            .write_all(signature)
            .chain_err(|| "writing signature file")?;
        signature_file
            .flush()
            .chain_err(|| "flushing signature file")?;

        // start verification
        let verify = Command::new("gpg")
            .arg("--homedir")
            .arg(gpgdir.path())
            .arg("--batch")
            // avoid warnings about untrusted keys
            .arg("--trust-model")
            .arg("tofu")
            .arg("--tofu-default-policy")
            .arg("good")
            .arg("--verify")
            .arg(&signature_path)
            .arg("-")
            .stdin(Stdio::piped())
            .spawn()
            .chain_err(|| "running gpg --verify")?;

        Ok(GpgReader {
            _gpgdir: gpgdir,
            source,
            child: verify,
        })
    }

    /// Read and discard all the bytes from the underlying reader, and
    /// verify the signature.
    pub fn consume(&mut self) -> Result<()> {
        let mut buf: [u8; 4096] = [0; 4096];
        while self.read(&mut buf).chain_err(|| "reading signed content")? > 0 {}
        Ok(())
    }
}

impl<R: Read> Read for GpgReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> result::Result<usize, io::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let count = self.source.read(buf)?;
        if count > 0 {
            // On a partial write we return an error in violation of the
            // API contract.  This should be okay, since it's a fatal error
            // for us anyway.
            self.child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(&buf[0..count])?;
        } else {
            // end of input; check result
            if !self.child.wait()?.success() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "GPG verification failure",
                ));
            }
        }
        Ok(count)
    }
}

impl<R: Read> Drop for GpgReader<R> {
    fn drop(&mut self) {
        // close stdin, reap process
        let _ = self.child.wait();
    }
}
