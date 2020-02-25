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
use nix::{self, ioctl_none, ioctl_read_bad, ioctl_write_ptr_bad, mount, request_code_none};
use regex::Regex;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::{remove_dir, File};
use std::io::{Seek, SeekFrom};
use std::num::NonZeroU32;
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use tempdir::TempDir;

use crate::errors::*;

pub fn mount_boot(device: &str) -> Result<Mount> {
    // get partition list
    let partitions = get_partitions(device)?;
    if partitions.is_empty() {
        bail!("couldn't find any partitions on {}", device);
    }

    // find the boot partition
    let boot_partitions = partitions
        .iter()
        .filter(|d| d.label.as_ref().unwrap_or(&"".to_string()) == "boot")
        .collect::<Vec<&BlkDev>>();
    let dev = match boot_partitions.len() {
        0 => bail!("couldn't find boot device for {}", device),
        1 => boot_partitions[0],
        _ => bail!("found multiple devices on {} with label \"boot\"", device),
    };

    // mount it
    match &dev.fstype {
        Some(fstype) => Mount::try_mount(&dev.path, &fstype),
        None => Err(format!("couldn't get filesystem type of boot device for {}", device).into()),
    }
}

#[derive(Debug)]
struct BlkDev {
    path: String,
    label: Option<String>,
    fstype: Option<String>,
}

fn get_partitions(device: &str) -> Result<Vec<BlkDev>> {
    // have lsblk enumerate partitions on the device
    let result = Command::new("lsblk")
        .arg("--pairs")
        .arg("--output")
        .arg("NAME,LABEL,FSTYPE")
        .arg(device)
        .output()
        .chain_err(|| "running lsblk")?;
    if !result.status.success() {
        // copy out its stderr
        eprint!("{}", String::from_utf8_lossy(&*result.stderr));
        bail!("lsblk of {} failed", device);
    }
    let output = String::from_utf8(result.stdout).chain_err(|| "decoding lsblk output")?;

    // walk each device in the output
    let mut result: Vec<BlkDev> = Vec::new();
    for line in output.lines() {
        // parse key-value pairs
        let fields = split_lsblk_line(line);

        // Older lsblk, e.g. in CentOS 7.6, doesn't support PATH.
        // Assemble device path from dirname and NAME.
        let mut path = Path::new(device)
            .parent()
            .chain_err(|| format!("couldn't get parent directory of {}", device))?
            .to_path_buf();
        match fields.get("NAME") {
            None => continue,
            Some(name) => path.push(name),
        }
        // Skip the device itself
        if path == Path::new(device) {
            continue;
        }

        result.push(BlkDev {
            path: path.to_str().expect("couldn't round-trip path").to_string(),
            label: fields.get("LABEL").map(|v| v.to_string()),
            fstype: fields.get("FSTYPE").map(|v| v.to_string()),
        });
    }
    Ok(result)
}

/// Parse key-value pairs from lsblk --pairs.
/// Newer versions of lsblk support JSON but the one in CentOS 7 doesn't.
fn split_lsblk_line(line: &str) -> HashMap<String, String> {
    let re = Regex::new(r#"([A-Z-]+)="([^"]+)""#).unwrap();
    let mut fields: HashMap<String, String> = HashMap::new();
    for cap in re.captures_iter(line) {
        fields.insert(cap[1].to_string(), cap[2].to_string());
    }
    fields
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
            match mount::umount(&self.mountpoint) {
                Ok(_) => break,
                Err(err) => {
                    if retries == 0 {
                        eprintln!("umounting {}: {}", self.device, err);
                        return;
                    } else {
                        sleep(Duration::from_millis(100));
                    }
                }
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
        match result {
            Ok(_) => break,
            Err(err) => {
                if retries == 0 {
                    if err == nix::Error::from_errno(Errno::EINVAL) {
                        return Err(err).chain_err(|| {
                            "couldn't reread partition table: device may not support partitions"
                        });
                    } else if err == nix::Error::from_errno(Errno::EBUSY) {
                        return Err(err)
                            .chain_err(|| "couldn't reread partition table: device is in use");
                    } else {
                        return Err(err).chain_err(|| "couldn't reread partition table");
                    }
                } else {
                    sleep(Duration::from_millis(100));
                }
            }
        }
    }
    Ok(())
}

/// Get the logical sector size of a block device.
pub fn get_sector_size(file: &mut File) -> Result<NonZeroU32> {
    let fd = file.as_raw_fd();
    let mut size: c_int = 0;
    match unsafe { blksszget(fd, &mut size) } {
        Ok(_) => {
            let size_u32: u32 = size
                .try_into()
                .chain_err(|| format!("sector size {} doesn't fit in u32", size))?;
            NonZeroU32::new(size_u32).ok_or_else(|| "found sector size of zero".into())
        }
        Err(e) => Err(Error::with_chain(e, "getting sector size")),
    }
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
ioctl_read_bad!(blksszget, request_code_none!(0x12, 104), c_int);
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

/// Inspect a buffer from the start of a disk image and return its formatted
/// sector size, if any can be determined.
pub fn detect_formatted_sector_size(buf: &[u8]) -> Option<NonZeroU32> {
    let gpt_magic: &[u8; 8] = b"EFI PART";

    if buf.len() >= 520 && buf[512..520] == gpt_magic[..] {
        // GPT at offset 512
        NonZeroU32::new(512)
    } else if buf.len() >= 4104 && buf[4096..4104] == gpt_magic[..] {
        // GPT at offset 4096
        NonZeroU32::new(4096)
    } else {
        // Unknown
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maplit::hashmap;
    use std::io::Read;
    use xz2::read::XzDecoder;

    #[test]
    fn lsblk_split() {
        assert_eq!(
            split_lsblk_line(r#"NAME="sda" LABEL="" FSTYPE="""#),
            hashmap! {
                String::from("NAME") => String::from("sda"),
            }
        );
        assert_eq!(
            split_lsblk_line(r#"NAME="sda1" LABEL="" FSTYPE="vfat""#),
            hashmap! {
                String::from("NAME") => String::from("sda1"),
                String::from("FSTYPE") => String::from("vfat")
            }
        );
        assert_eq!(
            split_lsblk_line(r#"NAME="sda2" LABEL="boot" FSTYPE="ext4""#),
            hashmap! {
                String::from("NAME") => String::from("sda2"),
                String::from("LABEL") => String::from("boot"),
                String::from("FSTYPE") => String::from("ext4"),
            }
        );
        assert_eq!(
            split_lsblk_line(r#"NAME="sda3" LABEL="foo=\x22bar\x22 baz" FSTYPE="ext4""#),
            hashmap! {
                String::from("NAME") => String::from("sda3"),
                // for now, we don't care about resolving lsblk's hex escapes,
                // so we just pass them through
                String::from("LABEL") => String::from(r#"foo=\x22bar\x22 baz"#),
                String::from("FSTYPE") => String::from("ext4"),
            }
        );
    }

    #[test]
    fn disk_sector_size_reader() {
        struct Test {
            name: &'static str,
            data: &'static [u8],
            compressed: bool,
            result: Option<NonZeroU32>,
        };
        let tests = vec![
            Test {
                name: "zero-length",
                data: b"",
                compressed: false,
                result: None,
            },
            Test {
                name: "empty-disk",
                data: include_bytes!("../fixtures/empty.xz"),
                compressed: true,
                result: None,
            },
            Test {
                name: "gpt-512",
                data: include_bytes!("../fixtures/gpt-512.xz"),
                compressed: true,
                result: NonZeroU32::new(512),
            },
            Test {
                name: "gpt-4096",
                data: include_bytes!("../fixtures/gpt-4096.xz"),
                compressed: true,
                result: NonZeroU32::new(4096),
            },
        ];

        for test in tests {
            let data = if test.compressed {
                let mut decoder = XzDecoder::new(test.data);
                let mut data: Vec<u8> = Vec::new();
                decoder.read_to_end(&mut data).expect("decompress failed");
                data
            } else {
                test.data.to_vec()
            };
            let result = detect_formatted_sector_size(&data);
            if result != test.result {
                panic!(
                    "\"{}\" returned incorrect result: expected {:?}, found {:?}",
                    test.name, test.result, result
                );
            }
        }
    }
}
