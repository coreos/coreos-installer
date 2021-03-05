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

use anyhow::{bail, Context, Result};
use std::fs::{metadata, set_permissions, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::result;
use tempfile::{self, TempDir};

pub struct GpgReader<R: Read> {
    _gpgdir: TempDir,
    source: R,
    child: Child,
}

impl<R: Read> GpgReader<R> {
    pub fn new(source: R, signature: &[u8]) -> Result<Self> {
        // create GPG home directory with restrictive mode
        let gpgdir = tempfile::Builder::new()
            .prefix("coreos-installer-")
            .tempdir()
            .context("creating temporary directory")?;
        let meta = metadata(gpgdir.path()).context("getting metadata for temporary directory")?;
        let mut permissions = meta.permissions();
        permissions.set_mode(0o700);
        set_permissions(gpgdir.path(), permissions)
            .context("setting mode for temporary directory")?;

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
            .context("running gpg --import")?;
        import
            .stdin
            .as_mut()
            .unwrap()
            .write_all(keys)
            .context("importing GPG keys")?;
        if !import.wait().context("waiting for gpg --import")?.success() {
            bail!("gpg --import failed");
        }

        // list the public keys we just imported
        let mut list = Command::new("gpg")
            .arg("--homedir")
            .arg(gpgdir.path())
            .arg("--batch")
            .arg("--list-keys")
            .arg("--with-colons")
            .stdout(Stdio::piped())
            .spawn()
            .context("running gpg --list-keys")?;
        let mut list_output = String::new();
        list.stdout
            .as_mut()
            .unwrap()
            .read_to_string(&mut list_output)
            .context("listing GPG keys")?;
        if !list
            .wait()
            .context("waiting for gpg --list-keys")?
            .success()
        {
            bail!("gpg --list-keys failed");
        }

        // accumulate key IDs into trust arguments
        let mut trust: Vec<&str> = Vec::new();
        for line in list_output.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            // only look at public keys
            if fields[0] != "pub" {
                continue;
            }
            // extract key ID
            if fields.len() >= 5 {
                trust.append(&mut vec!["--trusted-key", fields[4]]);
            }
        }

        // mark keys trusted in trustdb
        // We do this as a separate pass to keep the resulting log lines
        // out of the verify output.
        let trustdb = Command::new("gpg")
            .arg("--homedir")
            .arg(gpgdir.path())
            .arg("--batch")
            .arg("--check-trustdb")
            .args(trust)
            .output()
            .context("running gpg --check-trustdb")?;
        if !trustdb.status.success() {
            // copy out its stderr
            eprint!("{}", String::from_utf8_lossy(&*trustdb.stderr));
            bail!("gpg --check-trustdb failed");
        }

        // write signature to file
        let mut signature_path = gpgdir.path().to_path_buf();
        signature_path.push("signature");
        let mut signature_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&signature_path)
            .context("creating signature file")?;
        signature_file
            .write_all(signature)
            .context("writing signature file")?;

        // start verification
        let verify = Command::new("gpg")
            .arg("--homedir")
            .arg(gpgdir.path())
            .arg("--batch")
            .arg("--verify")
            .arg(&signature_path)
            .arg("-")
            .stdin(Stdio::piped())
            .spawn()
            .context("running gpg --verify")?;

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
        while self.read(&mut buf).context("reading signed content")? > 0 {}
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
