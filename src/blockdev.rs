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
use nix::sys::stat::{major, minor};
use nix::{errno::Errno, mount};
use regex::Regex;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::{metadata, read_dir, read_to_string, remove_dir, File, OpenOptions};
use std::num::{NonZeroU32, NonZeroU64};
use std::os::linux::fs::MetadataExt;
use std::os::raw::c_int;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use crate::errors::*;
use crate::io::resolve_link;

#[derive(Debug)]
pub struct Disk {
    pub path: String,
}

impl Disk {
    pub fn new(path: &str) -> Self {
        Disk {
            path: path.to_string(),
        }
    }

    pub fn mount_partition_by_label(&self, label: &str, allow_holder: bool, flags: mount::MsFlags) -> Result<Mount> {
        // get partition list
        let partitions = self.get_partitions(allow_holder)?;
        if partitions.is_empty() {
            bail!("couldn't find any partitions on {}", self.path);
        }

        // find the partition with the matching label
        let matching_partitions = partitions
            .iter()
            .filter(|d| d.label.as_ref().unwrap_or(&"".to_string()) == label)
            .collect::<Vec<&Partition>>();
        let part = match matching_partitions.len() {
            0 => bail!("couldn't find {} device for {}", label, self.path),
            1 => matching_partitions[0],
            _ => bail!(
                "found multiple devices on {} with label \"{}\"",
                self.path,
                label
            ),
        };

        // mount it
        match &part.fstype {
            Some(fstype) => Mount::try_mount(&part.path, &fstype, flags),
            None => Err(format!(
                "couldn't get filesystem type of {} device for {}",
                label, self.path
            )
            .into()),
        }
    }

    fn get_partitions(&self, with_holders: bool) -> Result<Vec<Partition>> {
        // have lsblk enumerate partitions on the device
        // Older lsblk, e.g. in CentOS 7.6, doesn't support PATH, but -p option
        let result = Command::new("lsblk")
            .arg("--pairs")
            .arg("--paths")
            .arg("--output")
            .arg("NAME,LABEL,FSTYPE,TYPE,MOUNTPOINT")
            .arg(&self.path)
            .output()
            .chain_err(|| "running lsblk")?;
        if !result.status.success() {
            // copy out its stderr
            eprint!("{}", String::from_utf8_lossy(&*result.stderr));
            bail!("lsblk of {} failed", &self.path);
        }
        let output = String::from_utf8(result.stdout).chain_err(|| "decoding lsblk output")?;

        // walk each device in the output
        let mut result: Vec<Partition> = Vec::new();
        for line in output.lines() {
            // parse key-value pairs
            let fields = split_lsblk_line(line);
            if let Some(name) = fields.get("NAME") {
                match fields.get("TYPE") {
                    // If unknown type, skip.
                    None => continue,
                    // If whole-disk device, skip.
                    Some(t) if t == &"disk".to_string() => continue,
                    // If partition, allow.
                    Some(t) if t == &"part".to_string() => (),
                    // If with_holders is true, allow anything else.
                    Some(_) if with_holders => (),
                    // Ignore LVM or RAID devices which are using one of the
                    // partitions but aren't a partition themselves.
                    _ => continue,
                };
                let (mountpoint, swap) = match fields.get("MOUNTPOINT") {
                    Some(mp) if mp == "[SWAP]" => (None, true),
                    Some(mp) => (Some(mp.to_string()), false),
                    None => (None, false),
                };
                result.push(Partition {
                    path: name.to_owned(),
                    label: fields.get("LABEL").map(<_>::to_string),
                    fstype: fields.get("FSTYPE").map(<_>::to_string),
                    parent: self.path.to_owned(),
                    mountpoint,
                    swap,
                });
            }
        }
        Ok(result)
    }

    /// Return an empty list if we have exclusive access to the device, or
    /// a list of partitions preventing us from gaining exclusive access.
    pub fn get_busy_partitions(self) -> Result<Vec<Partition>> {
        // Try rereading the partition table.  This is the most complete
        // check, but it only works on partitionable devices.
        let rereadpt_result = {
            let mut f = OpenOptions::new()
                .write(true)
                .open(&self.path)
                .chain_err(|| format!("opening {}", &self.path))?;
            reread_partition_table(&mut f).map(|_| Vec::new())
        };
        if rereadpt_result.is_ok() {
            return rereadpt_result;
        }

        // Walk partitions, record the ones that are reported in use,
        // and return the list if any
        let mut busy: Vec<Partition> = Vec::new();
        for d in self.get_partitions(false)? {
            if d.mountpoint.is_some() || d.swap || !d.get_holders()?.is_empty() {
                busy.push(d)
            }
        }
        if !busy.is_empty() {
            return Ok(busy);
        }

        // Our investigation found nothing.  If the device is expected to be
        // partitionable but reread failed, we evidently missed something,
        // so error out for safety
        if !self.is_dm_device() {
            return rereadpt_result;
        }

        Ok(Vec::new())
    }

    /// Get a handle to the set of device nodes for individual partitions
    /// of the device.
    pub fn get_partition_table(&self) -> Result<Box<dyn PartTable>> {
        if self.is_dm_device() {
            Ok(Box::new(PartTableKpartx::new(&self.path)?))
        } else {
            Ok(Box::new(PartTableKernel::new(&self.path)?))
        }
    }

    fn is_dm_device(&self) -> bool {
        self.path.starts_with("/dev/mapper/") || self.path.starts_with("/dev/dm-")
    }
}

/// A handle to the set of device nodes for individual partitions of a
/// device.  Must be held as long as the device nodes are needed; they might
/// be removed upon drop.
pub trait PartTable {
    /// Update device nodes for the current state of the partition table
    fn reread(&mut self) -> Result<()>;
}

/// Device nodes for partitionable kernel devices, managed by the kernel.
#[derive(Debug)]
pub struct PartTableKernel {
    path: String,
    file: File,
}

impl PartTableKernel {
    fn new(path: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .chain_err(|| format!("opening {}", path))?;
        Ok(Self {
            path: path.to_string(),
            file,
        })
    }
}

impl PartTable for PartTableKernel {
    fn reread(&mut self) -> Result<()> {
        reread_partition_table(&mut self.file)?;
        udev_settle()
    }
}

/// Device nodes for non-partitionable kernel devices, managed by running
/// kpartx to parse the partition table and create device-mapper devices for
/// each partition.
#[derive(Debug)]
pub struct PartTableKpartx {
    path: String,
    need_teardown: bool,
}

impl PartTableKpartx {
    fn new(path: &str) -> Result<Self> {
        let mut table = Self {
            path: path.to_string(),
            need_teardown: !Self::already_set_up(path)?,
        };
        // create/sync partition devices if missing
        table.reread()?;
        Ok(table)
    }

    // We only want to kpartx -d on drop if we're the one initially
    // creating the partition devices.  There's no good way to detect
    // this.
    fn already_set_up(path: &str) -> Result<bool> {
        let re = Regex::new(r"^p[0-9]+$").expect("compiling RE");
        let expected = Path::new(path)
            .file_name()
            .chain_err(|| format!("getting filename of {}", path))?
            .to_os_string()
            .into_string()
            .map_err(|_| format!("converting filename of {}", path))?;
        for ent in read_dir("/dev/mapper").chain_err(|| "listing /dev/mapper")? {
            let ent = ent.chain_err(|| "reading /dev/mapper entry")?;
            let found = ent.file_name().into_string().map_err(|_| {
                format!(
                    "converting filename of {}",
                    Path::new(&ent.file_name()).display()
                )
            })?;
            if found.starts_with(&expected) && re.is_match(&found[expected.len()..]) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn run_kpartx(&self, flag: &str) -> Result<()> {
        // Swallow stderr on success.  Avoids spurious warnings:
        //   GPT:Primary header thinks Alt. header is not at the end of the disk.
        //   GPT:Alternate GPT header not at the end of the disk.
        //   GPT: Use GNU Parted to correct GPT errors.
        //
        // By default, kpartx waits for udev to settle before returning,
        // but this blocks indefinitely inside a container.  See e.g.
        //   https://github.com/moby/moby/issues/22025
        // Use -n to skip blocking on udev, and then manually settle.
        let result = Command::new("kpartx")
            .arg(flag)
            .arg("-n")
            .arg(&self.path)
            .output()
            .chain_err(|| format!("running kpartx {} {}", flag, self.path))?;
        if !result.status.success() {
            // copy out its stderr
            eprint!("{}", String::from_utf8_lossy(&*result.stderr));
            bail!("kpartx {} {} failed: {}", flag, self.path, result.status);
        }
        udev_settle()?;
        Ok(())
    }
}

impl PartTable for PartTableKpartx {
    fn reread(&mut self) -> Result<()> {
        self.run_kpartx("-u")
    }
}

impl Drop for PartTableKpartx {
    /// If we created the partition devices (rather than finding them
    /// already existing), delete them afterward so we don't leave DM
    /// devices attached to the specified disk.
    fn drop(&mut self) {
        if self.need_teardown {
            if let Err(e) = self.run_kpartx("-d") {
                eprintln!("{}", e)
            }
        }
    }
}

#[derive(Debug)]
pub struct Partition {
    pub path: String,
    pub label: Option<String>,
    pub fstype: Option<String>,

    pub parent: String,
    pub mountpoint: Option<String>,
    pub swap: bool,
}

impl Partition {
    /// Return start and end offsets within the disk.
    pub fn get_offsets(path: &str) -> Result<(u64, u64)> {
        let dev = metadata(path)
            .chain_err(|| format!("getting metadata for {}", path))?
            .st_rdev();
        let maj: u64 = major(dev);
        let min: u64 = minor(dev);

        let start = read_sysfs_dev_block_value_u64(maj, min, "start")?;
        let size = read_sysfs_dev_block_value_u64(maj, min, "size")?;

        // We multiply by 512 here: the kernel values are always in 512 blocks, regardless of the
        // actual sector size of the block device. We keep the values as bytes to make things
        // easier.
        let start_offset: u64 = start
            .checked_mul(512)
            .ok_or_else(|| "start offset mult overflow")?;
        let end_offset: u64 = start_offset
            .checked_add(
                size.checked_mul(512)
                    .ok_or_else(|| "end offset mult overflow")?,
            )
            .ok_or_else(|| "end offset add overflow")?;
        Ok((start_offset, end_offset))
    }

    pub fn get_holders(&self) -> Result<Vec<String>> {
        let holders = self.get_sysfs_dir()?.join("holders");
        let mut ret: Vec<String> = Vec::new();
        for ent in read_dir(&holders).chain_err(|| format!("reading {}", &holders.display()))? {
            let ent = ent.chain_err(|| format!("reading {} entry", &holders.display()))?;
            ret.push(format!("/dev/{}", ent.file_name().to_string_lossy()));
        }
        Ok(ret)
    }

    // Try to locate the device directory in sysfs.
    fn get_sysfs_dir(&self) -> Result<PathBuf> {
        let basedir = Path::new("/sys/block");

        // First assume we have a regular partition.
        // /sys/block/sda/sda1
        let devdir = basedir
            .join(
                Path::new(&self.parent)
                    .file_name()
                    .chain_err(|| format!("parent {} has no filename", self.parent))?,
            )
            .join(
                Path::new(&self.path)
                    .file_name()
                    .chain_err(|| format!("path {} has no filename", self.path))?,
            );
        if devdir.exists() {
            return Ok(devdir);
        }

        // Now assume a kpartx "partition", where the path is a symlink to
        // an unpartitioned DM device node.
        // /sys/block/dm-1
        let (target, is_link) = resolve_link(&self.path)?;
        if is_link {
            let devdir = basedir.join(
                target
                    .file_name()
                    .chain_err(|| format!("target {} has no filename", target.display()))?,
            );
            if devdir.exists() {
                return Ok(devdir);
            }
        }

        // Give up
        bail!(
            "couldn't find /sys/block directory for partition {} of {}",
            &self.path,
            &self.parent
        );
    }
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
    fn try_mount(device: &str, fstype: &str, flags: mount::MsFlags) -> Result<Mount> {
        let tempdir = tempfile::Builder::new()
            .prefix("coreos-installer-")
            .tempdir()
            .chain_err(|| "creating temporary directory")?;
        // avoid auto-cleanup of tempdir, which could recursively remove
        // the partition contents if umount failed
        let mountpoint = tempdir.into_path();

        mount::mount::<str, Path, str, str>(Some(device), &mountpoint, Some(fstype), flags, None)
            .chain_err(|| format!("mounting device {} on {}", device, mountpoint.display()))?;

        Ok(Mount {
            device: device.to_string(),
            mountpoint,
        })
    }

    pub fn mountpoint(&self) -> &Path {
        self.mountpoint.as_path()
    }

    pub fn get_partition_offsets(&self) -> Result<(u64, u64)> {
        Partition::get_offsets(&self.device)
    }
}

fn read_sysfs_dev_block_value_u64(maj: u64, min: u64, field: &str) -> Result<u64> {
    let s = read_sysfs_dev_block_value(maj, min, field).chain_err(|| {
        format!(
            "reading partition {}:{} {} value from sysfs",
            maj, min, field
        )
    })?;
    Ok(s.parse().chain_err(|| {
        format!(
            "parsing partition {}:{} {} value \"{}\" as u64",
            maj, min, field, &s
        )
    })?)
}

fn read_sysfs_dev_block_value(maj: u64, min: u64, field: &str) -> Result<String> {
    let path = PathBuf::from(format!("/sys/dev/block/{}:{}/{}", maj, min, field));
    Ok(read_to_string(&path)?.trim_end().into())
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

fn reread_partition_table(file: &mut File) -> Result<()> {
    let fd = file.as_raw_fd();
    // Reread sometimes fails inexplicably.  Retry several times before
    // giving up.
    for retries in (0..20).rev() {
        let result = unsafe { ioctl::blkrrpart(fd) };
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

/// Get the sector size of the block device at a given path.
pub fn get_sector_size_for_path(device: &Path) -> Result<NonZeroU32> {
    let dev = OpenOptions::new()
        .read(true)
        .open(device)
        .chain_err(|| format!("opening {:?}", device))?;

    if !dev
        .metadata()
        .chain_err(|| format!("getting metadata for {:?}", device))?
        .file_type()
        .is_block_device()
    {
        bail!("{:?} is not a block device", device);
    }

    get_sector_size(&dev)
}

/// Get the logical sector size of a block device.
pub fn get_sector_size(file: &File) -> Result<NonZeroU32> {
    let fd = file.as_raw_fd();
    let mut size: c_int = 0;
    match unsafe { ioctl::blksszget(fd, &mut size) } {
        Ok(_) => {
            let size_u32: u32 = size
                .try_into()
                .chain_err(|| format!("sector size {} doesn't fit in u32", size))?;
            NonZeroU32::new(size_u32).ok_or_else(|| "found sector size of zero".into())
        }
        Err(e) => Err(Error::with_chain(e, "getting sector size")),
    }
}

/// Get the size of a block device.
pub fn get_block_device_size(file: &File) -> Result<NonZeroU64> {
    let fd = file.as_raw_fd();
    let mut size: libc::size_t = 0;
    match unsafe { ioctl::blkgetsize64(fd, &mut size) } {
        // just cast using `as`: there is no platform we care about today where size_t > 64bits
        Ok(_) => NonZeroU64::new(size as u64).ok_or_else(|| "found block size of zero".into()),
        Err(e) => Err(Error::with_chain(e, "getting block size")),
    }
}

// create unsafe ioctl wrappers
#[allow(clippy::missing_safety_doc)]
mod ioctl {
    use super::c_int;
    use nix::{ioctl_none, ioctl_read, ioctl_read_bad, request_code_none};
    ioctl_none!(blkrrpart, 0x12, 95);
    ioctl_read_bad!(blksszget, request_code_none!(0x12, 104), c_int);
    ioctl_read!(blkgetsize64, 0x12, 114, libc::size_t);
}

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
