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

use error_chain::bail;
use regex::RegexBuilder;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use walkdir::WalkDir;

use crate::errors::*;

/////////////////////////////////////////////////////////////////////////////
// IBM Z Bootloader Support
/////////////////////////////////////////////////////////////////////////////

/// Generate zipl config and set boot device
///
/// # Arguments
/// * `boot` - path to boot partition mountpoint, i.e. smth like /boot
/// * `disk` - smth like /dev/dasda
pub fn install_bootloader<P: AsRef<Path>>(boot: P, disk: &str) -> Result<()> {
    eprintln!("Installing bootloader");

    let bls_config_path = get_bls_config_path(&boot)?;
    let kernel_path = get_kernel_path(&boot)?;
    let initramfs_path = get_initramfs_path(&boot)?;

    let kargs = format!("{} ignition.firstboot", get_boot_kargs(bls_config_path)?);

    let status = Command::new("zipl")
        .arg("-P")
        .arg(kargs)
        .arg("-i")
        .arg(kernel_path.as_os_str())
        .arg("-r")
        .arg(initramfs_path.as_os_str())
        .arg("--target")
        .arg(boot.as_ref().as_os_str())
        .arg("-n")
        .stdout(Stdio::null())
        .status()
        .chain_err(|| format!("failed to execute zipl on {}", disk))?;
    if !status.success() {
        bail!("couldn't install bootloader on {}", disk);
    }

    eprintln!("Updating re-IPL device");
    let status = Command::new("chreipl")
        .arg(disk)
        .stdout(Stdio::null())
        .status()
        .chain_err(|| format!("failed to execute chreipl on {}", disk))?;
    if !status.success() {
        bail!("couldn't set {} as boot device", disk);
    }
    Ok(())
}

fn find_file<P: AsRef<Path>>(root: P, pat: &str) -> Result<PathBuf> {
    for entry in WalkDir::new(root.as_ref())
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if pat.starts_with('.') {
            if !name.ends_with(pat) {
                continue;
            }
        } else if !name.starts_with(pat) {
            continue;
        }

        return Ok(entry.path().to_path_buf());
    }
    bail!(
        "Cannot find file with mask: {} in {}",
        pat,
        root.as_ref().display()
    )
}

fn get_bls_config_path<P: AsRef<Path>>(boot: P) -> Result<PathBuf> {
    find_file(boot.as_ref().join("loader").join("entries"), ".conf")
}

fn get_kernel_path<P: AsRef<Path>>(boot: P) -> Result<PathBuf> {
    find_file(boot.as_ref().join("ostree"), "vmlinuz")
}

fn get_initramfs_path<P: AsRef<Path>>(boot: P) -> Result<PathBuf> {
    find_file(boot.as_ref().join("ostree"), "initram")
}

fn get_boot_kargs<P: AsRef<Path>>(bls_config: P) -> Result<String> {
    let contents = read_to_string(&bls_config)
        .chain_err(|| format!("reading {}", bls_config.as_ref().display()))?;
    // read kargs from options line
    let pt = r"^options (?P<v>.+)$";
    let opts = RegexBuilder::new(pt)
        .multi_line(true)
        .build()
        .unwrap()
        .captures(&contents)
        .chain_err(|| format!("capturing {:?}", pt))?
        .name("v")
        .map(|v| v.as_str())
        .chain_err(|| format!("matching {:?}", pt))?;
    // filter out variable substitutions such as $ignition_firstboot
    let opts = RegexBuilder::new(r"(^| )\$[a-zA-Z0-9_]+")
        .build()
        .unwrap()
        .replace_all(opts, "")
        .to_string();
    Ok(opts)
}
