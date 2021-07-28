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
use std::fs::{copy, create_dir_all, read_dir};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::Builder;

/////////////////////////////////////////////////////////////////////////////
// IBM Z Bootloader Support
/////////////////////////////////////////////////////////////////////////////

/// Generate zipl config and set boot device
///
/// # Arguments
/// * `boot` - path to boot partition mountpoint, i.e. smth like /boot
/// * `disk` - optional device path on which to run chreipl
/// * `firstboot` - adds ignition.firstboot karg
/// * `firstboot_kargs` - enables ignition firstboot and adds provided kargs
pub fn install_bootloader<P: AsRef<Path>>(
    boot: P,
    disk: Option<&str>,
    firstboot: bool,
    firstboot_kargs: Option<&str>,
) -> Result<()> {
    eprintln!("Installing bootloader");
    let boot = boot.as_ref();

    run_zipl(boot, firstboot, firstboot_kargs)?;

    if let Some(disk) = disk {
        eprintln!("Updating re-IPL device");
        runcmd!("chreipl", disk)?;
    }
    Ok(())
}

fn run_zipl<P: AsRef<Path>>(
    boot: P,
    firstboot: bool,
    firstboot_kargs: Option<&str>,
) -> Result<()> {
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
    let blsdir = if firstboot || firstboot_kargs.is_some() {
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
        let mut extra = if firstboot {
            vec!["ignition.firstboot".to_string()]
        } else {
            Vec::new()
        };
        if let Some(kargs) = firstboot_kargs {
            extra.extend_from_slice(
                &kargs
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
