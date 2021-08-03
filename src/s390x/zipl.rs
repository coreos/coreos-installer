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

use crate::install::{bls_entry_options_delete_and_append_kargs, visit_bls_entry_options};
use crate::runcmd;
use anyhow::{anyhow, Context, Result};
use regex::Regex;
use std::fs::{copy, create_dir_all, read_dir};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::Builder;

/// Sets the boot device to `dev` using `chreipl`.
pub fn chreipl<P: AsRef<Path>>(dev: P) -> Result<()> {
    eprintln!("Updating re-IPL device");
    runcmd!("chreipl", dev.as_ref())?;
    Ok(())
}

/// Runs `zipl` based on Ignition and BLS configuration in `boot`.
pub fn zipl<P: AsRef<Path>>(boot: P) -> Result<()> {
    let boot = boot.as_ref();

    // create dummy config for zipl
    let mut conffile = Builder::new()
        .prefix("coreos-installer-zipl.")
        .tempfile()
        .context("creating zipl config")?;
    let data = format!(
        "[defaultboot]\ndefaultauto\nprompt=1\ntimeout=5\nsecure=auto\ntarget={}\n",
        boot.to_str().unwrap()
    );
    conffile
        .write_all(data.as_bytes())
        .context("writing zipl config")?;

    // we have to copy bls config files for further modification
    let tempdir = Builder::new()
        .prefix("coreos-installer-zipl-bls-")
        .tempdir()
        .context("creating temporary directory")?;
    let firstboot_file = boot.join("ignition.firstboot");
    let blsdir = if firstboot_file.exists() {
        let blsdir = tempdir.path().join("loader/entries");
        create_dir_all(&blsdir).with_context(|| format!("creating {}", blsdir.display()))?;
        read_dir(boot.join("loader/entries"))
            .with_context(|| format!("reading {}", boot.display()))?
            .into_iter()
            .filter_map(Result::ok)
            .filter(|p| p.file_type().unwrap().is_file())
            .for_each(|src| {
                copy(src.path(), blsdir.join(src.file_name())).unwrap();
            });
        let mut extra = vec!["ignition.firstboot".to_string()];
        let firstboot_contents = std::fs::read_to_string(&firstboot_file)
            .with_context(|| format!("reading \"{}\"", firstboot_file.display()))?;
        if let Some(firstboot_kargs) = extract_firstboot_kargs(&firstboot_contents)? {
            extra.extend_from_slice(
                &firstboot_kargs
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>(),
            );
        }
        visit_bls_entry_options(tempdir.path(), |orig_options: &str| {
            bls_entry_options_delete_and_append_kargs(orig_options, &[], &[], extra.as_slice())
        })
        .with_context(|| format!("appending {:?}", extra))?;

        blsdir
    } else {
        boot.join("loader/entries")
    };

    runcmd!("zipl", "--blsdir", blsdir, "--config", conffile.path())?;

    Ok(())
}

/// Returns the first-boot kargs embedded in the contents `s` of a firstboot file.
///
/// Note this isn't intended to be a general purpose GRUB config language parser. Only the exact
/// format used by coreos-installer is recognized. Any other format triggers an error.
fn extract_firstboot_kargs(s: &str) -> Result<Option<String>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }

    let captures = Regex::new(r#"^set ignition_network_kcmdline="([^\n]*)"$"#)
        .expect("compiling RE")
        .captures(&s)
        .context("couldn't parse kargs from ignition.firstboot file")?;
    match captures.get(1).expect("kargs").as_str() {
        "" => Ok(None), // this shouldn't really happen, but be nice
        kargs => Ok(Some(kargs.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_firstboot_kargs() {
        assert_eq!(extract_firstboot_kargs("").unwrap(), None);
        assert_eq!(extract_firstboot_kargs("\n").unwrap(), None);
        assert_eq!(
            extract_firstboot_kargs("set ignition_network_kcmdline=\"\"").unwrap(),
            None
        );
        assert_eq!(
            extract_firstboot_kargs("set ignition_network_kcmdline=\"foobar\"").unwrap(),
            Some("foobar".into())
        );
        assert_eq!(
            extract_firstboot_kargs("\nset ignition_network_kcmdline=\"foobar\"\n\n").unwrap(),
            Some("foobar".into())
        );
        assert_eq!(
            extract_firstboot_kargs("set ignition_network_kcmdline=\"foo bar\"").unwrap(),
            Some("foo bar".into())
        );
        assert!(extract_firstboot_kargs("set ignition_network_kcmdline=\"\n\"").is_err());
        assert!(extract_firstboot_kargs(
            "set ignition_network_kcmdline=\"\"\nset ignition_network_kcmdline=\"\""
        )
        .is_err());
        assert!(extract_firstboot_kargs("stuff\nset ignition_network_kcmdline=\"\"").is_err());
        assert!(extract_firstboot_kargs("set ignition_network_kcmdline=\"\"\nstuff").is_err());
        assert!(extract_firstboot_kargs("set ignition_network_kcmdline=\"foo\nbar\"").is_err());
        assert!(extract_firstboot_kargs("foobar").is_err());
    }
}
