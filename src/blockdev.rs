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
use nix::errno::Errno;
use nix::{self, ioctl_none, ioctl_write_ptr_bad, mount, request_code_none};
use serde::Deserialize;
use std::fs::{remove_dir, File};
use std::io::{Seek, SeekFrom};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use tempdir::TempDir;

use crate::errors::*;

pub fn mount_boot(device: &str) -> Result<Mount> {
    let dev = get_partition_with_label(device, "boot")?
        .chain_err(|| format!("couldn't find boot device for {}", device))?;
    match dev.fstype {
        Some(fstype) => Mount::try_mount(&dev.path, &fstype),
        None => Err(format!("couldn't get filesystem type of boot device for {}", device).into()),
    }
}

#[derive(Deserialize)]
struct LsBlk {
    blockdevices: Vec<BlkDev>,
}

#[derive(Deserialize)]
struct BlkDev {
    path: String,
    label: Option<String>,
    fstype: Option<String>,
}

fn get_partition_with_label(device: &str, label: &str) -> Result<Option<BlkDev>> {
    let result = Command::new("lsblk")
        .arg("--json")
        .arg("--output")
        .arg("PATH,LABEL,FSTYPE")
        .arg(device)
        .output()
        .chain_err(|| "running lsblk")?;
    if !result.status.success() {
        // copy out its stderr
        eprint!("{}", String::from_utf8_lossy(&*result.stderr));
        bail!("lsblk of {} failed", device);
    }
    let output: LsBlk =
        serde_json::from_slice(&*result.stdout).chain_err(|| "decoding lsblk JSON")?;
    let mut found: Option<BlkDev> = None;
    for dev in output.blockdevices {
        if dev.label.is_none() || dev.label.as_ref().unwrap() != label {
            continue;
        }
        if found.is_some() {
            bail!("found multiple devices on {} with label: {}", device, label);
        }
        found = Some(dev);
    }
    Ok(found)
}

#[derive(Debug)]
pub struct Mount {
    device: String,
    mountpoint: PathBuf,
}

impl Mount {
    fn try_mount(device: &str, fstype: &str) -> Result<Mount> {
        let tempdir =
            TempDir::new("coreos-installer").chain_err(|| "creating temporary directory")?;
        // avoid auto-cleanup of tempdir, which could recursively remove
        // the partition contents if umount failed
        let mountpoint = tempdir.into_path();

        mount::mount::<str, Path, str, str>(
            Some(device),
            &mountpoint,
            Some(fstype),
            mount::MsFlags::empty(),
            None,
        )
        .chain_err(|| format!("mounting device {} on {}", device, mountpoint.display()))?;

        Ok(Mount {
            device: device.to_string(),
            mountpoint,
        })
    }

    pub fn mountpoint(&self) -> &Path {
        self.mountpoint.as_path()
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        // Unmount sometimes fails immediately after closing the last open
        // file on the partition.  Retry several times before giving up.
        for retries in (0..20).rev() {
            let result = mount::umount(&self.mountpoint);
            if result.is_ok() {
                break;
            } else if retries == 0 {
                eprintln!("umounting {}: {}", self.device, result.unwrap_err());
                return;
            } else {
                sleep(Duration::from_millis(100));
            }
        }
        if let Err(err) = remove_dir(&self.mountpoint) {
            eprintln!("removing {}: {}", self.mountpoint.display(), err);
            return;
        }
    }
}

pub fn reread_partition_table(file: &mut File) -> Result<()> {
    let fd = file.as_raw_fd();
    // Reread sometimes fails inexplicably.  Retry several times before
    // giving up.
    for retries in (0..20).rev() {
        let result = unsafe { blkrrpart(fd) };
        if result.is_ok() {
            break;
        } else if retries == 0 {
            return result.and(Ok(())).chain_err(|| "rereading partition table");
        } else {
            sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

/// Try discarding all blocks from the underlying block device.
/// Return true if successful, false if the underlying device doesn't
/// support discard, or an error otherwise.
pub fn try_discard_all(file: &mut File) -> Result<bool> {
    // get device size
    let length = file
        .seek(SeekFrom::End(0))
        .chain_err(|| "seeking device file")?;
    file.seek(SeekFrom::Start(0))
        .chain_err(|| "seeking device file")?;

    // discard
    let fd = file.as_raw_fd();
    let range: [u64; 2] = [0, length];
    match unsafe { blkdiscard(fd, &range) } {
        Ok(_) => Ok(true),
        Err(e) => {
            if e == nix::Error::from_errno(Errno::EOPNOTSUPP) {
                Ok(false)
            } else {
                Err(Error::with_chain(e, "discarding device contents"))
            }
        }
    }
}

// create unsafe ioctl wrappers
ioctl_none!(blkrrpart, 0x12, 95);
ioctl_write_ptr_bad!(blkdiscard, request_code_none!(0x12, 119), [u64; 2]);

pub fn udev_settle() -> Result<()> {
    // "udevadm settle" silently no-ops if the udev socket is missing, and
    // then lsblk can't find partition labels.  Catch this early.
    if !Path::new("/run/udev/control").exists() {
        return Err(
            "udevd socket missing; are we running in a container without /run/udev mounted?".into(),
        );
    }

    // There's a potential window after rereading the partition table where
    // udevd hasn't yet received updates from the kernel, settle will return
    // immediately, and lsblk won't pick up partition labels.  Try to sleep
    // our way out of this.
    sleep(Duration::from_millis(200));

    let status = Command::new("udevadm")
        .arg("settle")
        .status()
        .chain_err(|| "running udevadm settle")?;
    if !status.success() {
        bail!("udevadm settle failed");
    }
    Ok(())
}
