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
use std::thread::{self, JoinHandle};
use tempfile::{self, TempDir};

#[derive(Debug)]
pub enum VerifyKeys {
    /// Production keys
    Production,
    /// Snake oil key
    #[cfg(test)]
    InsecureTest,
}

#[derive(Debug)]
enum VerifyReport {
    /// Report verification result to stderr
    Stderr,
    /// Report verification result to stderr only if successful
    StderrOnSuccess,
    /// Verify silently
    Ignore,
}

pub struct VerifyReader<R: Read> {
    typ: VerifyType<R>,
}

enum VerifyType<R: Read> {
    None(R),
    Gpg(GpgReader<R>),
}

impl<R: Read> VerifyReader<R> {
    pub fn new(source: R, gpg_signature: Option<&[u8]>, keys: VerifyKeys) -> Result<Self> {
        let typ = if let Some(signature) = gpg_signature {
            VerifyType::Gpg(GpgReader::new(source, signature, keys)?)
        } else {
            VerifyType::None(source)
        };
        Ok(VerifyReader { typ })
    }

    /// Return an error if signature verification fails, and report the
    /// result to stderr
    pub fn verify(&mut self) -> Result<()> {
        match &mut self.typ {
            VerifyType::None(_) => (),
            VerifyType::Gpg(reader) => reader.finish(VerifyReport::Stderr)?,
        }
        Ok(())
    }

    /// Return an error if signature verification fails.  Report the result
    /// to stderr if verification is successful, but not if it fails.
    pub fn verify_without_logging_failure(&mut self) -> Result<()> {
        match &mut self.typ {
            VerifyType::None(_) => (),
            VerifyType::Gpg(reader) => reader.finish(VerifyReport::StderrOnSuccess)?,
        }
        Ok(())
    }
}

impl<R: Read> Read for VerifyReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.typ {
            VerifyType::None(reader) => reader.read(buf),
            VerifyType::Gpg(reader) => reader.read(buf),
        }
    }
}

struct GpgReader<R: Read> {
    _gpgdir: TempDir,
    source: R,
    child: Child,
    stderr_thread: Option<JoinHandle<io::Result<Vec<u8>>>>,
}

impl<R: Read> GpgReader<R> {
    fn new(source: R, signature: &[u8], keys: VerifyKeys) -> Result<Self> {
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
        let keys = match keys {
            VerifyKeys::Production => &include_bytes!("../signing-keys.asc")[..],
            #[cfg(test)]
            VerifyKeys::InsecureTest => {
                &include_bytes!("../../fixtures/verify/test-key.pub.asc")[..]
            }
        };
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
        let mut verify = Command::new("gpg")
            .arg("--homedir")
            .arg(gpgdir.path())
            .arg("--batch")
            .arg("--verify")
            .arg(&signature_path)
            .arg("-")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("running gpg --verify")?;

        // spawn stderr reader
        let mut stderr = verify.stderr.take().unwrap();
        let stderr_thread = thread::Builder::new()
            .name("gpg-stderr".into())
            .spawn(move || -> io::Result<Vec<u8>> {
                let mut buf = Vec::new();
                stderr.read_to_end(&mut buf)?;
                Ok(buf)
            })
            .context("spawning GPG stderr reader")?;

        Ok(GpgReader {
            _gpgdir: gpgdir,
            source,
            child: verify,
            stderr_thread: Some(stderr_thread),
        })
    }

    /// Stop GPG, forward its stderr if requested, and check its exit status.
    /// The exit status check happens on every call, but stderr forwarding
    /// only happens on the first call.
    fn finish(&mut self, report: VerifyReport) -> io::Result<()> {
        // do cleanup first: wait for child process and join on thread
        let wait_result = self.child.wait();
        let join_result = self.stderr_thread.take().map(|t| t.join());

        // possibly copy GPG's stderr to ours
        let success = wait_result?.success();
        match join_result {
            // thread returned GPG's stderr
            Some(Ok(Ok(stderr))) => match report {
                VerifyReport::StderrOnSuccess if !success => (),
                // use eprint rather than io::stderr() so the output is
                // captured when running tests
                VerifyReport::Stderr | VerifyReport::StderrOnSuccess => {
                    eprint!("{}", String::from_utf8_lossy(&stderr))
                }
                VerifyReport::Ignore => (),
            },
            // thread returned error
            Some(Ok(Err(e))) => return Err(e),
            // thread panicked; propagate the panic
            Some(Err(e)) => std::panic::resume_unwind(e),
            // already joined the thread on a previous call
            None => (),
        }

        // check GPG exit status
        if !success {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "GPG verification failure",
            ));
        }

        Ok(())
    }
}

impl<R: Read> Read for GpgReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
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
        }
        Ok(count)
    }
}

impl<R: Read> Drop for GpgReader<R> {
    fn drop(&mut self) {
        // if we haven't already forwarded GPG's stderr, avoid doing it now,
        // so we don't imply that we're checking the result
        self.finish(VerifyReport::Ignore).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read data with valid signature
    #[test]
    fn test_good_signature() {
        let data = include_bytes!("../../fixtures/verify/test-key.priv.asc");
        let sig = include_bytes!("../../fixtures/verify/test-key.priv.asc.sig");

        let mut reader =
            VerifyReader::new(&data[..], Some(&sig[..]), VerifyKeys::InsecureTest).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        reader.verify().unwrap();
        reader.verify().unwrap();
        reader.verify_without_logging_failure().unwrap();
        assert_eq!(&buf[..], &data[..]);
    }

    /// Read data with bad signature
    #[test]
    fn test_bad_signature() {
        let mut data = include_bytes!("../../fixtures/verify/test-key.priv.asc").clone();
        let sig = include_bytes!("../../fixtures/verify/test-key.priv.asc.sig");
        data[data.len() - 1] = b'!';

        let mut reader =
            VerifyReader::new(&data[..], Some(&sig[..]), VerifyKeys::InsecureTest).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        reader.verify().unwrap_err();
        reader.verify().unwrap_err();
        reader.verify_without_logging_failure().unwrap_err();
        assert_eq!(&buf[..], &data[..]);
    }

    /// Read truncated data with otherwise-valid signature
    #[test]
    fn test_truncated_data() {
        let data = include_bytes!("../../fixtures/verify/test-key.priv.asc");
        let sig = include_bytes!("../../fixtures/verify/test-key.priv.asc.sig");

        let mut reader =
            VerifyReader::new(&data[..1000], Some(&sig[..]), VerifyKeys::InsecureTest).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        reader.verify().unwrap_err();
        reader.verify().unwrap_err();
        reader.verify_without_logging_failure().unwrap_err();
        assert_eq!(&buf[..], &data[..1000]);
    }

    /// Read data with signing key not in keyring
    #[test]
    fn test_no_pubkey() {
        let data = include_bytes!("../../fixtures/verify/test-key.priv.asc");
        let sig = include_bytes!("../../fixtures/verify/test-key.priv.asc.random.sig");

        let mut reader =
            VerifyReader::new(&data[..], Some(&sig[..]), VerifyKeys::InsecureTest).unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        reader.verify().unwrap_err();
        reader.verify().unwrap_err();
        reader.verify_without_logging_failure().unwrap_err();
        assert_eq!(&buf[..], &data[..]);
    }
}
