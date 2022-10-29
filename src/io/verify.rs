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
use pipe::{pipe, PipeReader, PipeWriter};
use sequoia_openpgp::cert::CertParser;
use sequoia_openpgp::parse::stream::{
    DetachedVerifierBuilder, MessageLayer, MessageStructure, VerificationError, VerificationHelper,
};
use sequoia_openpgp::parse::{PacketParser, Parse};
use sequoia_openpgp::policy::StandardPolicy;
use sequoia_openpgp::{Cert, KeyHandle};
use std::io::{self, Read, Write};
use std::thread::{self, JoinHandle};

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
    source: R,
    verify_pipe: Option<PipeWriter>,
    verify_thread: Option<JoinHandle<Result<String>>>,
    success: bool,
}

impl<R: Read> GpgReader<R> {
    fn new(source: R, signature: &[u8], keys: VerifyKeys) -> Result<Self> {
        // parse public keys
        let helper = GpgHelper::new(
            CertParser::from(
                PacketParser::from_bytes(match keys {
                    VerifyKeys::Production => &include_bytes!("../signing-keys.asc")[..],
                    #[cfg(test)]
                    VerifyKeys::InsecureTest => {
                        &include_bytes!("../../fixtures/verify/test-key.pub.asc")[..]
                    }
                })
                .context("decoding verification keys")?,
            )
            .collect::<Result<Vec<Cert>>>()
            .context("parsing verification keys")?,
        );

        // start verification
        fn verify(reader: PipeReader, signature: Vec<u8>, helper: GpgHelper) -> Result<String> {
            let policy = StandardPolicy::new();
            let mut verifier = DetachedVerifierBuilder::from_bytes(&signature)
                .context("parsing signature")?
                .with_policy(&policy, None, helper)
                .context("creating signature verifier")?;
            verifier
                .verify_reader(reader)
                .map(|_| verifier.into_helper().success_detail.unwrap())
        }
        let (pipe_read, pipe_write) = pipe();
        let sig = signature.to_vec();
        let verify_thread = thread::Builder::new()
            .name("gpg-verify".into())
            .spawn(move || verify(pipe_read, sig, helper))
            .context("spawning GPG verifier")?;

        Ok(GpgReader {
            source,
            verify_pipe: Some(pipe_write),
            verify_thread: Some(verify_thread),
            success: false,
        })
    }

    /// Stop GPG, forward its stderr if requested, and check its exit status.
    /// The exit status check happens on every call, but stderr forwarding
    /// only happens on the first call.
    fn finish(&mut self, report: VerifyReport) -> Result<()> {
        // if the thread hasn't been cleaned up, collect results
        if let Some(thread) = self.verify_thread.take() {
            // close pipe
            self.verify_pipe.take();
            // wait for thread
            let result = match thread.join() {
                // thread returned normally
                Ok(res) => res,
                // thread panicked; propagate the panic
                Err(e) => std::panic::resume_unwind(e),
            };
            // record result
            self.success = result.is_ok();

            // report result to stderr if enabled
            match report {
                VerifyReport::StderrOnSuccess if !self.success => (),
                // use eprintln rather than io::stderr() so the output is
                // captured when running tests
                VerifyReport::Stderr | VerifyReport::StderrOnSuccess => match result {
                    Ok(s) => eprintln!("{}", s),
                    Err(e) => eprintln!("{}", e),
                },
            }
        }

        // return result
        if !self.success {
            bail!("GPG verification failure");
        }
        Ok(())
    }
}

impl<R: Read> Read for GpgReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.verify_pipe.is_none() {
            return Ok(0);
        }
        let count = self.source.read(buf)?;
        if count > 0 {
            // On a partial write we return an error in violation of the
            // API contract.  This should be okay, since it's a fatal error
            // for us anyway.
            self.verify_pipe
                .as_mut()
                .unwrap()
                .write_all(&buf[0..count])?;
        }
        Ok(count)
    }
}

struct GpgHelper {
    certs: Vec<Cert>,
    success_detail: Option<String>,
}

impl GpgHelper {
    fn new(certs: Vec<Cert>) -> Self {
        Self {
            certs,
            success_detail: None,
        }
    }
}

impl VerificationHelper for GpgHelper {
    fn get_certs(&mut self, _ids: &[KeyHandle]) -> Result<Vec<Cert>> {
        Ok(self.certs.clone())
    }

    fn check(&mut self, s: MessageStructure) -> Result<()> {
        if s.len() != 1 {
            bail!("wrong number of layers ({}) in message structure", s.len());
        }
        if let MessageLayer::SignatureGroup { ref results } = s[0] {
            let mut errs = Vec::new();
            for res in results {
                use VerificationError::*;
                match res {
                    // XXX improve these
                    Ok(_) => {
                        self.success_detail = Some(format!(
                            "Good signature from \"{}\"\n    made on {} with key {}",
                            "a", "b", "c"
                        ));
                        return Ok(());
                    }
                    Err(MalformedSignature { error, .. }) => {
                        errs.push(format!("Malformed signature: {}", error));
                    }
                    Err(MissingKey { .. }) => {
                        errs.push("Missing key".to_string());
                    }
                    Err(UnboundKey { error, .. }) => {
                        errs.push(format!("Unbound key: {}", error));
                    }
                    Err(BadKey { error, .. }) => {
                        errs.push(format!("Bad key: {}", error));
                    }
                    Err(BadSignature { error, .. }) => {
                        errs.push(format!("Bad signature: {}", error));
                    }
                }
            }
            if !errs.is_empty() {
                bail!(errs.join("\n"));
            }
        }
        bail!("couldn't find any signatures");
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

    /// Read data with garbage signature
    #[test]
    fn test_garbage_signature() {
        let data = include_bytes!("../../fixtures/verify/test-key.priv.asc").clone();
        let sig = b"asdf";

        let mut reader =
            VerifyReader::new(&data[..], Some(&sig[..]), VerifyKeys::InsecureTest).unwrap();
        let mut buf = Vec::new();
        // verifier thread exits early on parse error
        assert!(matches!(
            reader.read_to_end(&mut buf).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        ));
        reader.verify().unwrap_err();
        reader.verify().unwrap_err();
        reader.verify_without_logging_failure().unwrap_err();
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
