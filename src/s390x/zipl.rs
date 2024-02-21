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
use crate::io::{visit_bls_entry, Initrd};
use crate::s390x::ZiplSecexMode;
use crate::util::cmd_output;
use crate::{runcmd, runcmd_output};
use anyhow::{anyhow, bail, Context, Result};
use lazy_static::lazy_static;
use nix::mount::MsFlags;
use regex::Regex;
use std::fs::{read_dir, DirEntry, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{Builder, NamedTempFile};

/// Sets the boot device to `dev` using `chreipl`.
pub fn chreipl<P: AsRef<Path>>(dev: P) -> Result<()> {
    eprintln!("Updating re-IPL device");
    runcmd!("chreipl", dev.as_ref())?;
    Ok(())
}

/// Secure boot (Secure IPL) includes support of SCSI and ECKD DASD boot devices
enum Loaddev {
    Eckd(String),
    Scsi(String, String, String),
}

fn parse_lszdev_eckd(line: &str) -> Result<Loaddev> {
    // ECKD ID looks like: 0.0.5223, we need only last part of it (5223)
    lazy_static! {
        static ref REGEX: Regex = Regex::new(r#"[[:digit:]].[[:digit:]].([[:xdigit:]]+)"#).unwrap();
    }
    if let Some(cap) = REGEX.captures_iter(line).next() {
        return Ok(Loaddev::Eckd(cap[1].to_string()));
    }
    bail!("bad ECKD id: {}", line);
}

fn parse_lszdev_zfcp(line: &str) -> Result<Loaddev> {
    // SCSI ID looks like: 0.0.8000:0x500507630400d1e3:0x4000401d00000000
    // So here is regex to parse required ids: 8000,500507630400d1e3,4000401d00000000
    lazy_static! {
        static ref REGEX: Regex = Regex::new(
            r#"[[:digit:]].[[:digit:]].([[:xdigit:]]+):0x([[:xdigit:]]+):0x([[:xdigit:]]+)"#
        )
        .unwrap();
    }
    if let Some(cap) = REGEX.captures_iter(line).next() {
        return Ok(Loaddev::Scsi(
            cap[1].to_string(),
            cap[2].to_string(),
            cap[3].to_string(),
        ));
    }
    bail!("bad zFCP id: {}", line);
}

fn parse_lszdev<P: AsRef<Path>>(dev: P) -> Result<Loaddev> {
    // We don't want to traverse sysfs and do same stuff lszdev does,
    // so just call it to get required info. Here is sample output:
    // $ lszdev -c TYPE,ID -n
    // dasd-eckd    0.0.0190
    // zfcp-lun     0.0.8007:0x500507630400d1e3:0x4001404c00000000
    // qeth         0.0.bdd0:0.0.bdd1:0.0.bdd2
    // generic-ccw  0.0.000c
    let output = runcmd_output!(
        "lszdev",
        "-n",
        "--columns",
        "TYPE,ID",
        "--by-node",
        dev.as_ref()
    )?;
    let (devtype, id) = output
        .trim()
        .split_once(' ')
        .with_context(|| format!("parsing lszdev {output}"))?;
    match devtype {
        "dasd-eckd" => parse_lszdev_eckd(id),
        "zfcp-lun" => parse_lszdev_zfcp(id),
        _ => bail!("unsupported device: {} id: {}", devtype, id),
    }
}

/// Sets zVM Secure Boot (Secure IPL) boot device to `dev`.
pub fn set_loaddev<P: AsRef<Path>>(dev: P) -> Result<()> {
    if !secure_ipl_is_supported()? {
        bail!("Secure IPL is not supported");
    }
    // check if system is zVM guest
    if !Path::new("/dev/vmcp").exists() {
        return Ok(());
    }
    eprintln!("Setting LOADDEV");
    let mut cmd = Command::new("vmcp");
    cmd.arg("set").arg("loaddev");
    match parse_lszdev(dev)? {
        Loaddev::Eckd(d) => cmd.arg("eckd").arg("dev").arg(d),
        Loaddev::Scsi(d, p, l) =>
        // CP tool wants portname/lun to be splitted at 8th character:
        // $ vmcp set loaddev dev 8007 portname 500507630400d1e3 lun 4001404c00000000 secure
        // HCPZPM002E Invalid operand - 500507630400D1E3
        // $ vmcp set loaddev dev 8007 portname 50050763 0400d1e3 lun 4001404c 00000000 secure
        {
            cmd.arg("dev")
                .arg(d)
                .arg("portname")
                .arg(&p[0..8])
                .arg(&p[8..])
                .arg("lun")
                .arg(&l[0..8])
                .arg(&l[8..])
        }
    };
    cmd.arg("secure");
    cmd_output(&mut cmd)?;
    Ok(())
}

fn secure_execution_is_enabled() -> Result<bool> {
    sysfs_flag_enabled("/sys/firmware/uv/prot_virt_guest")
}

fn secure_ipl_is_supported() -> Result<bool> {
    sysfs_flag_enabled("/sys/firmware/ipl/has_secure")
}

fn sysfs_flag_enabled<P: AsRef<Path>>(path: P) -> Result<bool> {
    match File::open(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.as_ref().display())),
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
        // we have to take care of ignition.firstboot karg
        let firstboot_file = boot.join("ignition.firstboot");
        let (kernel, initrd, mut options) = get_info_from_bls(boot)?;
        // we need a full path to kernel and initrd
        let kernel = boot.join(&kernel[1..]);
        let initrd = boot.join(&initrd[1..]);

        if firstboot_file.exists() {
            options.push_str(" ignition.firstboot");
            let firstboot_contents = std::fs::read_to_string(&firstboot_file)
                .with_context(|| format!("reading \"{}\"", firstboot_file.display()))?;
            if let Some(firstboot_kargs) = extract_firstboot_kargs(&firstboot_contents)? {
                options = format!("{options} {firstboot_kargs}");
            }
        }
        if let Some(kargs) = kargs {
            options = format!("{options} {kargs}");
        }

        runcmd!(
            "zipl",
            "-V",
            "--target",
            boot,
            "--image",
            kernel,
            "--ramdisk",
            initrd,
            "--parameters",
            options
        )
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
