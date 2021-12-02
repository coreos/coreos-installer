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

use anyhow::{bail, Context, Result};
use nix::mount;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use libcoreinst::blockdev::*;
use libcoreinst::io::*;
use libcoreinst::util::*;

use libcoreinst::runcmd_output;

use crate::cmdline::*;

pub fn rootmap(config: RootmapConfig) -> Result<()> {
    // get the backing device for the root mount
    let mount = Mount::from_existing(&config.root_mount)?;
    let device = PathBuf::from(mount.device());

    // and from that we can collect all the parent backing devices too
    let mut backing_devices = get_blkdev_deps_recursing(&device)?;
    backing_devices.push(device);

    // for each of those, convert them to kargs
    let mut kargs = Vec::new();
    for backing_device in backing_devices {
        if let Some(dev_kargs) = device_to_kargs(&mount, backing_device)? {
            kargs.extend(dev_kargs);
        }
    }

    // we push the root kargs last, this has the nice property that the final order of kargs goes
    // from lowest level to highest; see also
    // https://github.com/coreos/fedora-coreos-tracker/issues/465
    kargs.push(format!("root=UUID={}", mount.get_filesystem_uuid()?));

    // we need this because with root= it's systemd that takes care of mounting via
    // systemd-fstab-generator, and it defaults to read-only otherwise
    kargs.push("rw".into());

    let rootflags = runcmd_output!("coreos-rootflags", &config.root_mount)?;
    let rootflags = rootflags.trim();
    if !rootflags.is_empty() {
        kargs.push(format!("rootflags={}", rootflags));
    }

    let boot_mount = get_boot_mount_from_cmdline_args(&config.boot_mount, &config.boot_device)?;
    if let Some(mount) = boot_mount {
        visit_bls_entry_options(mount.mountpoint(), |orig_options: &str| {
            KargsEditor::new()
                .append(&kargs)
                .maybe_apply_to(orig_options)
        })
        .context("appending rootmap kargs")?;
        eprintln!("Injected kernel arguments into BLS: {}", kargs.join(" "));
        // Note here we're not calling `zipl` on s390x; it will be called anyway on firstboot by
        // `coreos-ignition-firstboot-complete.service`, so might as well batch them.
    } else {
        // without /boot options, we just print the kargs; note we output to stdout here
        println!("{}", kargs.join(" "));
    }

    Ok(())
}

// This is shared with the kargs code -- might move this to a helper file eventually
pub fn get_boot_mount_from_cmdline_args(
    boot_mount: &Option<String>,
    boot_device: &Option<String>,
) -> Result<Option<Mount>> {
    if let Some(path) = boot_mount {
        Ok(Some(Mount::from_existing(path)?))
    } else if let Some(devpath) = boot_device {
        let devinfo = lsblk_single(Path::new(devpath))?;
        let fs = devinfo
            .get("FSTYPE")
            .with_context(|| format!("failed to query filesystem for {}", devpath))?;
        Ok(Some(Mount::try_mount(
            devpath,
            fs,
            mount::MsFlags::empty(),
        )?))
    } else {
        Ok(None)
    }
}

fn device_to_kargs(root: &Mount, device: PathBuf) -> Result<Option<Vec<String>>> {
    let blkinfo = lsblk_single(&device)?;
    let blktype = blkinfo
        .get("TYPE")
        .with_context(|| format!("missing TYPE for {}", device.display()))?;
    // a `match {}` construct would be nice here, but for RAID it's a prefix match
    if blktype.starts_with("raid") || blktype == "linear" {
        Ok(Some(get_raid_kargs(&device)?))
    } else if blktype == "crypt" {
        Ok(Some(get_luks_kargs(root, &device)?))
    } else if blktype == "part" || blktype == "disk" || blktype == "mpath" {
        Ok(None)
    } else {
        bail!("unknown block device type {}", blktype)
    }
}

fn get_raid_kargs(device: &Path) -> Result<Vec<String>> {
    let details = mdadm_detail(device)?;
    let uuid = details
        .get("MD_UUID")
        .with_context(|| format!("missing MD_UUID for {}", device.display()))?;
    Ok(vec![format!("rd.md.uuid={}", uuid)])
}

fn mdadm_detail(device: &Path) -> Result<HashMap<String, String>> {
    let output = runcmd_output!("mdadm", "--detail", "--export", device)?;
    let mut result: HashMap<String, String> = HashMap::new();
    for line in output.lines() {
        let (key, val) = split_mdadm_line(line)?;
        result.insert(key, val);
    }
    Ok(result)
}

fn split_mdadm_line(line: &str) -> Result<(String, String)> {
    let v: Vec<&str> = line.splitn(2, '=').collect();
    if v.len() != 2 {
        bail!("invalid mdadm line: {}", line);
    }
    Ok((v[0].into(), v[1].into()))
}

fn get_luks_kargs(root: &Mount, device: &Path) -> Result<Vec<String>> {
    // The LUKS UUID is a property of the backing block device of *this* block device, so we have
    // to get its parent. This is a bit awkward because we're already iterating through parents, so
    // theoretically we could re-use the same state here. But meh... this is easier to understand.
    let deps = get_blkdev_deps(device)?;
    match deps.len() {
        0 => bail!("missing parent device for {}", device.display()),
        1 => {
            let uuid = get_luks_uuid(&deps[0])?;
            let name = get_luks_name(device)?;
            let mut kargs = vec![format!("rd.luks.name={}={}", uuid, name)];
            if crypttab_device_has_netdev(root, &name)? {
                kargs.push("rd.neednet=1".into());
                kargs.push("rd.luks.options=_netdev".into());
            }
            Ok(kargs)
        }
        _ => bail!(
            "found multiple parent devices for crypt device {}",
            device.display()
        ),
    }
}

// crypttab is the source of truth for whether an encrypted block device requires networking.
fn crypttab_device_has_netdev(root: &Mount, dmname: &str) -> Result<bool> {
    let crypttab_path = root.mountpoint().join("etc/crypttab");

    let crypttab = std::fs::read_to_string(&crypttab_path)
        .with_context(|| format!("opening {}", crypttab_path.display()))?;
    for line in crypttab.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();

        // from crypttab(5), format is:
        //   name encrypted-device password options
        // first two fields are mandatory, remaining are optional

        if fields.len() < 2 {
            bail!("crypttab line missing name or device: {}", line);
        }

        if fields[0] != dmname {
            continue;
        }

        if fields.len() < 4 {
            return Ok(false);
        }
        return Ok(fields[3].split(',').any(|opt| opt == "_netdev"));
    }

    bail!("couldn't find {} in {}", dmname, crypttab_path.display());
}

fn get_luks_name(device: &Path) -> Result<String> {
    Ok(runcmd_output!(
        "dmsetup",
        "info",
        "--columns",
        "--noheadings",
        "-o",
        "name",
        device
    )?
    .trim()
    .into())
}

fn get_luks_uuid(device: &Path) -> Result<String> {
    Ok(runcmd_output!("cryptsetup", "luksUUID", device)?
        .trim()
        .into())
}

pub fn bind_boot(config: BindBootConfig) -> Result<()> {
    let boot_mount = Mount::from_existing(&config.boot_mount)?;
    let root_mount = Mount::from_existing(&config.root_mount)?;
    let boot_uuid = boot_mount.get_filesystem_uuid()?;
    let root_uuid = root_mount.get_filesystem_uuid()?;

    let kargs = vec![format!("boot=UUID={}", boot_uuid)];
    let changed = visit_bls_entry_options(boot_mount.mountpoint(), |orig_options: &str| {
        if !orig_options.starts_with("boot=") && !orig_options.contains(" boot=") {
            KargsEditor::new()
                .append(&kargs)
                .maybe_apply_to(orig_options)
        } else {
            // boot= karg already exists; let's not add anything
            Ok(None)
        }
    })
    .context("appending boot kargs")?;

    // put it in /run also for the first boot real root mount
    // https://github.com/coreos/fedora-coreos-config/blob/8661649009/overlay.d/05core/usr/lib/systemd/system-generators/coreos-boot-mount-generator#L105-L108
    if changed {
        let boot_uuid_run = Path::new("/run/coreos/bootfs_uuid");
        let parent = boot_uuid_run.parent().unwrap();
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        std::fs::write(boot_uuid_run, format!("{}\n", &boot_uuid))
            .with_context(|| format!("writing {}", boot_uuid_run.display()))?;
    }

    // bind rootfs to bootfs
    let root_uuid_stamp = boot_mount.mountpoint().join(".root_uuid");
    if root_uuid_stamp.exists() {
        let bound_root_uuid = std::fs::read_to_string(&root_uuid_stamp)
            .with_context(|| format!("reading {}", root_uuid_stamp.display()))?;
        let bound_root_uuid = bound_root_uuid.trim();
        // Let it slide if it already matches the rootfs... that shouldn't happen unless the user
        // is trying to force a rerun of Ignition. In that case, we'll have nicer errors and
        // warnings elsewhere.
        if bound_root_uuid != root_uuid {
            bail!(
                "boot filesystem already bound to a root filesystem (UUID: {})",
                bound_root_uuid
            );
        }
    } else {
        std::fs::write(&root_uuid_stamp, format!("{}\n", root_uuid))
            .with_context(|| format!("writing {}", root_uuid_stamp.display()))?;
    }

    // now bind GRUB to bootfs
    #[cfg(not(target_arch = "s390x"))]
    {
        let grub_bios_path = boot_mount.mountpoint().join("grub2/bootuuid.cfg");
        write_boot_uuid_grub2_dropin(&boot_uuid, grub_bios_path)?;
    }

    for esp in find_colocated_esps(boot_mount.device())? {
        let mount = Mount::try_mount(&esp, "vfat", mount::MsFlags::empty())?;
        let vendor_dir = find_efi_vendor_dir(&mount)?;
        let grub_efi_path = vendor_dir.join("bootuuid.cfg");
        write_boot_uuid_grub2_dropin(&boot_uuid, grub_efi_path)?;
    }
    Ok(())
}

fn write_boot_uuid_grub2_dropin<P: AsRef<Path>>(uuid: &str, p: P) -> Result<()> {
    let p = p.as_ref();
    std::fs::write(p, format!("set BOOT_UUID=\"{}\"\n", uuid))
        .with_context(|| format!("writing {}", p.display()))
}
