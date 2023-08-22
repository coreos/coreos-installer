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

use crate::blockdev::Mount;
use crate::io::{visit_bls_entry, visit_bls_entry_options, Initrd, KargsEditor};
use crate::s390x::ZiplSecexMode;
use crate::util::cmd_output;
use crate::{runcmd, runcmd_output};
use anyhow::{anyhow, Context, Result};
use nix::mount::MsFlags;
use regex::Regex;
use std::fs::{copy, create_dir_all, read_dir, DirEntry, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{Builder, NamedTempFile};

/// Sets the boot device to `dev` using `chreipl`.
pub fn chreipl<P: AsRef<Path>>(dev: P) -> Result<()> {
    let vm = runcmd_output!("systemd-detect-virt")?;
    if vm == "zvm" {
        eprintln!("Updating re-IPL device");
        runcmd!("chreipl", dev.as_ref())?;
    }
    Ok(())
}

fn secure_execution_is_enabled() -> Result<bool> {
    let sysfs_flag = "/sys/firmware/uv/prot_virt_guest";
    match File::open(sysfs_flag) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("reading {sysfs_flag}")),
        Ok(mut f) => {
            let mut buffer = String::new();
            f.read_to_string(&mut buffer)?;
            Ok(buffer.trim() == "1")
        }
    }
}

fn find_files<P: AsRef<Path>>(
    path: P,
    f: impl Fn(&DirEntry) -> Result<bool>,
) -> Result<Vec<PathBuf>> {
    read_dir(&path)
        .with_context(|| format!("reading directory {}", path.as_ref().display()))?
        .filter_map(|r| {
            r.map_err(anyhow::Error::new)
                .and_then(|ent| f(&ent).map(|b| b.then(|| ent.path())))
                .transpose()
        })
        .collect::<Result<Vec<_>>>()
}

fn generate_initrd<P: AsRef<Path>>(source: P, files: &[PathBuf]) -> Result<NamedTempFile> {
    let source = source.as_ref();
    let mut dest = Builder::new()
        .prefix("initrd")
        .suffix(".img")
        .append(true)
        .tempfile()
        .context("creating cpio for extras")?;

    // copying original initrd to tmpfile
    let mut initrd = File::open(source)?;
    std::io::copy(&mut initrd, &mut dest)
        .with_context(|| format!("copying {} to {}", source.display(), dest.path().display()))?;

    let mut initrd = Initrd::default();
    for path in files {
        let contents =
            std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let path = path
            .to_str()
            .with_context(|| format!("path {} is not UTF-8", path.display()))?;
        initrd.add(path, contents);
    }

    // appending
    let initrd = initrd.to_bytes()?;
    dest.as_file_mut()
        .write(&initrd)
        .with_context(|| format!("appending luks-initrd to {}", dest.path().display()))?;
    Ok(dest)
}

fn get_info_from_bls(boot: &Path) -> Result<(String, String, String)> {
    let mut kernel = None;
    let mut initrd = None;
    let mut options = None;

    let read_opts = |contents: &str| {
        for l in contents.lines() {
            match l.split_once(' ') {
                Some(("linux", s)) => kernel = Some(s.trim().to_owned()),
                Some(("initrd", s)) => initrd = Some(s.trim().to_owned()),
                Some(("options", s)) => options = Some(s.trim().to_owned()),
                _ => {}
            }
        }
        Ok(None)
    };
    visit_bls_entry(boot, read_opts)?;

    let kernel = kernel.ok_or_else(|| anyhow!("missing 'linux' key in default BLS config"))?;
    let initrd = initrd.ok_or_else(|| anyhow!("missing 'initrd' key in default BLS config"))?;
    let options = options.ok_or_else(|| anyhow!("missing 'options' key in default BLS config"))?;

    Ok((kernel, initrd, options))
}

fn generate_sdboot(
    mountpoint: &Path,
    boot: &Path,
    hostkey: Option<String>,
    kargs: Option<String>,
    files: Option<Vec<String>>,
) -> Result<PathBuf> {
    let (kernel, initrd, mut options) = get_info_from_bls(boot)?;

    // we need a full path to kernel and initrd
    let kernel = boot.join(&kernel[1..]);
    let initrd = boot.join(&initrd[1..]);

    // write all kargs to a tmpfile, so genprotimg can append them to sd-boot
    if let Some(kargs) = kargs {
        options = format!("{options} {kargs}");
    }
    let mut cmdline = Builder::new()
        .prefix("se-cmdline.")
        .tempfile()
        .context("creating zipl se cmdline")?;
    cmdline
        .write_all(options.as_bytes())
        .context("writing zipl se cmdline")?;

    let mut appendies = files.map_or_else(Vec::new, |v| v.iter().map(PathBuf::from).collect());

    let lukskeys_path = PathBuf::from("/etc/luks");
    let crypttab_path = PathBuf::from("/etc/crypttab");
    if lukskeys_path.exists() && crypttab_path.exists() {
        let mut keys = find_files(&lukskeys_path, |e: &DirEntry| Ok(e.metadata()?.is_file()))?;
        appendies.append(&mut keys);
        appendies.push(crypttab_path);
    };

    // Generate new initrd only when we append smth
    let new_initrd = if appendies.is_empty() {
        None
    } else {
        Some(generate_initrd(&initrd, &appendies)?)
    };

    let initrd = new_initrd.as_ref().map(|v| v.path()).unwrap_or(&initrd);

    // during cosa-build we override hostkey(s) with a universal one
    let hostkeys = if let Some(hostkey) = hostkey {
        vec![PathBuf::from(hostkey)]
    } else {
        find_files("/etc/se-hostkeys", |e: &DirEntry| {
            Ok(e.file_name()
                .to_str()
                .map(|p| p.starts_with("ibm-z-hostkey-"))
                .unwrap_or_default())
        })?
    };

    // finally, Secure Execution sd-boot image
    let sdboot = mountpoint.join("sdboot");
    let mut cmd = Command::new("genprotimg");
    cmd.arg("-V")
        .arg("-i")
        .arg(kernel)
        .arg("-r")
        .arg(initrd)
        .arg("-p")
        .arg(cmdline.path())
        .arg("--no-verify")
        .arg("-o")
        .arg(&sdboot);
    for k in hostkeys {
        cmd.arg("-k").arg(k);
    }
    cmd_output(&mut cmd)?;
    Ok(sdboot)
}

/// Runs `zipl` based on Ignition and BLS configuration in `boot`.
pub fn zipl<P: AsRef<Path>>(
    boot: P,
    hostkey: Option<String>,
    kargs: Option<String>,
    mode: ZiplSecexMode,
    files: Option<Vec<String>>,
) -> Result<()> {
    let boot = boot.as_ref();

    let secex = match mode {
        ZiplSecexMode::Auto => secure_execution_is_enabled()?,
        ZiplSecexMode::Enforce => true,
        ZiplSecexMode::Disable => false,
    };

    if secex {
        // Secure Execution is only supported with pre-built qemu-secex image
        let target = Mount::try_mount("/dev/disk/by-label/se", "ext4", MsFlags::empty())?;
        let sdboot = generate_sdboot(target.mountpoint(), boot, hostkey, kargs, files)?;

        runcmd!(
            "zipl",
            "-V",
            "--target",
            target.mountpoint(),
            "--image",
            sdboot
        )
    } else {
        // This branch could be also executed during installation, that's why
        // we have to take care of ignition.firstboot karg and copy bls config
        // files for further modification
        let tempdir = Builder::new()
            .prefix("coreos-installer-zipl-bls-")
            .tempdir()
            .context("creating temporary directory")?;
        let firstboot_file = boot.join("ignition.firstboot");
        let blsdir = if kargs.is_some() || firstboot_file.exists() {
            let blsdir = tempdir.path().join("loader/entries");
            create_dir_all(&blsdir).with_context(|| format!("creating {}", blsdir.display()))?;
            read_dir(boot.join("loader/entries"))
                .with_context(|| format!("reading {}", boot.display()))?
                .filter_map(Result::ok)
                .filter(|p| p.file_type().unwrap().is_file())
                .for_each(|src| {
                    copy(src.path(), blsdir.join(src.file_name())).unwrap();
                });

            let mut extra = Vec::new();
            if firstboot_file.exists() {
                extra.push("ignition.firstboot".to_string());
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
            }
            if let Some(kargs) = kargs {
                extra.extend_from_slice(
                    &kargs
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<String>>(),
                );
            }

            visit_bls_entry_options(tempdir.path(), |orig_options: &str| {
                KargsEditor::new()
                    .append_if_missing(extra.as_slice())
                    .maybe_apply_to(orig_options)
            })
            .with_context(|| format!("appending {extra:?}"))?;

            blsdir
        } else {
            boot.join("loader/entries")
        };

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

        runcmd!("zipl", "--blsdir", blsdir, "--config", conffile.path())
    }
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
        .captures(s)
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
