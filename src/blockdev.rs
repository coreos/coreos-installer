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
use gptman::{GPTPartitionEntry, GPT};
use nix::sys::stat::{major, minor};
use nix::{errno::Errno, mount};
use regex::Regex;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::{
    canonicalize, metadata, read_dir, read_to_string, remove_dir, symlink_metadata, File,
    OpenOptions,
};
use std::io::{Read, Seek, SeekFrom};
use std::num::{NonZeroU32, NonZeroU64};
use std::os::linux::fs::MetadataExt;
use std::os::raw::c_int;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use uuid::Uuid;

use crate::cmdline::PartitionFilter;
use crate::errors::*;
use crate::util::*;

use crate::{runcmd, runcmd_output};

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

    pub fn mount_partition_by_label(
        &self,
        label: &str,
        allow_holder: bool,
        flags: mount::MsFlags,
    ) -> Result<Mount> {
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
        // walk each device in the output
        let mut result: Vec<Partition> = Vec::new();
        for devinfo in lsblk(Path::new(&self.path))? {
            if let Some(name) = devinfo.get("NAME") {
                match devinfo.get("TYPE") {
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
                let (mountpoint, swap) = match devinfo.get("MOUNTPOINT") {
                    Some(mp) if mp == "[SWAP]" => (None, true),
                    Some(mp) => (Some(mp.to_string()), false),
                    None => (None, false),
                };
                result.push(Partition {
                    path: name.to_owned(),
                    label: devinfo.get("LABEL").map(<_>::to_string),
                    fstype: devinfo.get("FSTYPE").map(<_>::to_string),
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
        runcmd_output!("kpartx", flag, "-n", &self.path)?;
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
        let is_link = symlink_metadata(&self.path)
            .chain_err(|| format!("reading metadata for {}", self.path))?
            .file_type()
            .is_symlink();
        if is_link {
            let target = canonicalize(&self.path)
                .chain_err(|| format!("getting absolute path to {}", self.path))?;
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

#[derive(Debug)]
pub struct Mount {
    device: String,
    mountpoint: PathBuf,
    /// Whether we own this mount.
    owned: bool,
}

impl Mount {
    pub fn try_mount(device: &str, fstype: &str, flags: mount::MsFlags) -> Result<Mount> {
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
            owned: true,
        })
    }

    pub fn from_existing(path: &str) -> Result<Mount> {
        let mounts = read_to_string("/proc/self/mounts").chain_err(|| "reading mount table")?;
        for line in mounts.lines() {
            let mount: Vec<&str> = line.split_whitespace().collect();
            // see https://man7.org/linux/man-pages/man5/fstab.5.html
            if mount.len() != 6 {
                bail!("invalid line in /proc/self/mounts: {}", line);
            }
            if mount[1] == path {
                return Ok(Mount {
                    device: mount[0].to_string(),
                    mountpoint: path.into(),
                    owned: false,
                });
            }
        }
        bail!("mountpoint {} not found", path);
    }

    pub fn device(&self) -> &str {
        self.device.as_str()
    }

    pub fn mountpoint(&self) -> &Path {
        self.mountpoint.as_path()
    }

    pub fn get_partition_offsets(&self) -> Result<(u64, u64)> {
        Partition::get_offsets(&self.device)
    }

    pub fn get_filesystem_uuid(&self) -> Result<String> {
        let devinfo = lsblk_single(Path::new(&self.device))?;
        devinfo
            .get("UUID")
            .map(String::from)
            .chain_err(|| format!("filesystem {} has no UUID", self.device))
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        if !self.owned {
            return;
        }

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

#[derive(Debug)]
pub struct SavedPartitions {
    sector_size: u64,
    partitions: Vec<(u32, GPTPartitionEntry)>,
}

impl SavedPartitions {
    /// Create a SavedPartitions for a block device with a sector size.
    pub fn new_from_disk(disk: &mut File, filters: &[PartitionFilter]) -> Result<Self> {
        if !disk
            .metadata()
            .chain_err(|| "getting disk metadata")?
            .file_type()
            .is_block_device()
        {
            bail!("specified file is not a block device");
        }
        Self::new(disk, get_sector_size(&disk)?.get() as u64, filters)
    }

    /// Create a SavedPartitions for a file with a specified imputed sector
    /// size.  Useful for unit tests, and fails on a real disk.
    #[cfg(test)]
    pub fn new_from_file(
        disk: &mut File,
        sector_size: u64,
        filters: &[PartitionFilter],
    ) -> Result<Self> {
        if disk
            .metadata()
            .chain_err(|| "getting disk metadata")?
            .file_type()
            .is_block_device()
        {
            bail!("called new_from_file() on a block device");
        }
        match sector_size {
            512 | 4096 => (),
            _ => bail!("specified unreasonable sector size {}", sector_size),
        }
        Self::new(disk, sector_size, filters)
    }

    fn new(disk: &mut File, sector_size: u64, filters: &[PartitionFilter]) -> Result<Self> {
        // read GPT
        let gpt = match GPT::find_from(disk) {
            Ok(gpt) => gpt,
            Err(gptman::Error::InvalidSignature) => {
                // no GPT on this disk, so no partitions to save
                return Ok(Self {
                    sector_size,
                    partitions: Vec::new(),
                });
            }
            Err(e) => return Err(e).chain_err(|| "reading partition table"),
        };

        // cross-check GPT sector size
        Self::verify_gpt_sector_size(&gpt, sector_size)?;

        // save partitions accepted by filters
        let mut partitions = Vec::new();
        if !filters.is_empty() {
            for (i, p) in gpt.iter() {
                if Self::matches_filters(i, p, filters) {
                    partitions.push((i, p.clone()));
                }
            }
        }
        let result = Self {
            sector_size,
            partitions,
        };

        // Test restoring the saved partitions to a temporary file.  If the
        // resulting partition table contains invalid data (e.g. duplicate
        // partition GUIDs) we need to know now, before the caller
        // overwrites the partition table.  Otherwise we could fail to
        // restore, clear the table, and fail to restore _again_ to the
        // empty table.
        if !result.partitions.is_empty() {
            let len = disk
                .seek(SeekFrom::End(0))
                .chain_err(|| "getting disk size")?;
            let mut temp = tempfile::tempfile().chain_err(|| "creating dry run image")?;
            temp.set_len(len)
                .chain_err(|| format!("setting test image size to {}", len))?;
            result.overwrite(&mut temp).chain_err(|| {
                "failed dry run restoring saved partitions; input partition table may be invalid"
            })?;
        }

        Ok(result)
    }

    fn verify_disk_sector_size(&self, disk: &File) -> Result<()> {
        if !disk
            .metadata()
            .chain_err(|| "getting disk metadata")?
            .file_type()
            .is_block_device()
        {
            return Ok(());
        }
        let disk_sector_size = get_sector_size(&disk)?.get() as u64;
        if disk_sector_size != self.sector_size {
            bail!(
                "disk sector size {} doesn't match expected {}",
                disk_sector_size,
                self.sector_size
            );
        }
        Ok(())
    }

    fn verify_gpt_sector_size(gpt: &GPT, sector_size: u64) -> Result<()> {
        if gpt.sector_size != sector_size {
            bail!(
                "GPT sector size {} doesn't match expected {}",
                gpt.sector_size,
                sector_size
            );
        }
        Ok(())
    }

    fn matches_filters(i: u32, p: &GPTPartitionEntry, filters: &[PartitionFilter]) -> bool {
        use PartitionFilter::*;
        if !p.is_used() {
            return false;
        }
        filters.iter().any(|f| match f {
            Index(Some(first), _) if first.get() > i => false,
            Index(_, Some(last)) if last.get() < i => false,
            Index(_, _) => true,
            Label(glob) if glob.matches(p.partition_name.as_str()) => true,
            _ => false,
        })
    }

    /// Unconditionally write the saved partitions, and only the saved
    /// partitions, to the disk.  Updating the kernel partition table is the
    /// caller's responsibility.
    pub fn overwrite(&self, disk: &mut File) -> Result<()> {
        // create GPT
        self.verify_disk_sector_size(disk)?;
        let mut gpt = GPT::new_from(disk, self.sector_size, *Uuid::new_v4().as_bytes())
            .chain_err(|| "creating new GPT")?;

        // add partitions
        for (i, p) in &self.partitions {
            gpt[*i] = p.clone();
        }

        // write
        gpt.write_into(disk).chain_err(|| "writing new GPT")?;
        GPT::write_protective_mbr_into(disk, self.sector_size)
            .chain_err(|| "writing protective MBR")?;

        Ok(())
    }

    /// If any partitions are saved, merge them into the GPT from source,
    /// which must be valid.  Updating the kernel partition table is the
    /// caller's responsibility.
    pub fn merge(&self, source: &mut (impl Read + Seek), disk: &mut File) -> Result<()> {
        if self.partitions.is_empty() {
            return Ok(());
        }

        // read GPT
        self.verify_disk_sector_size(disk)?;
        let mut gpt =
            GPT::find_from(source).chain_err(|| "couldn't read partition table from source")?;
        Self::verify_gpt_sector_size(&gpt, self.sector_size)?;

        // Fail if the last on-disk partition overlaps with the beginning of
        // the first saved partition.  Ignore holes.  This test is distinct
        // from the download-time LimitReader checking, because the image
        // may claim to have partitions beyond the end of the image file.
        // If this occurs, install() will restore the saved partitions after
        // clearing the table.
        if let Some((i_end, end)) = gpt.iter().max_by_key(|(_, p)| p.ending_lba) {
            if let Some((i_start, start)) =
                self.partitions.iter().min_by_key(|(_, p)| p.starting_lba)
            {
                if end.ending_lba >= start.starting_lba {
                    bail!(
                        "disk partition {} ('{}') ends after start of saved partition {} ('{}')",
                        i_end,
                        end.partition_name.as_str(),
                        i_start,
                        start.partition_name.as_str()
                    )
                }
            }
        }

        // merge saved partitions into partition table
        // find partition number one larger than the largest used one
        let mut next = gpt
            .iter()
            .fold(1, |prev, (i, e)| if e.is_used() { i + 1 } else { prev });
        for (i, p) in &self.partitions {
            // use the next partition number in the sequence if we have to,
            // or the partition's original number if it's larger
            next = next.max(*i);
            eprintln!(
                "Saving partition {} (\"{}\") to new partition {}",
                i, p.partition_name, next
            );
            gpt[next] = p.clone();
            next += 1;
        }

        // write
        gpt.write_into(disk).chain_err(|| "writing updated GPT")?;

        Ok(())
    }

    /// Get the byte offset of the first byte not to be overwritten, if any,
    /// plus a description of the partition at that offset.
    pub fn get_offset(&self) -> Result<Option<(u64, String)>> {
        match self.partitions.iter().min_by_key(|(_, p)| p.starting_lba) {
            None => Ok(None),
            Some((i, p)) => Ok(Some((
                p.starting_lba
                    .checked_mul(self.sector_size)
                    .chain_err(|| "overflow calculating partition start")?,
                format!("partition {} (\"{}\")", i, p.partition_name.as_str()),
            ))),
        }
    }

    pub fn is_saved(&self) -> bool {
        !self.partitions.is_empty()
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

pub fn lsblk_single(dev: &Path) -> Result<HashMap<String, String>> {
    let mut devinfos = lsblk(Path::new(dev))?;
    if devinfos.is_empty() {
        // this should never happen because `lsblk` itself would've failed
        bail!("no lsblk results for {}", dev.display());
    }
    Ok(devinfos.remove(0))
}

pub fn lsblk(dev: &Path) -> Result<Vec<HashMap<String, String>>> {
    // Older lsblk, e.g. in CentOS 7.6, doesn't support PATH, but --paths option
    let output = runcmd_output!(
        "lsblk",
        "--pairs",
        "--paths",
        "--output",
        "NAME,LABEL,FSTYPE,TYPE,MOUNTPOINT,UUID",
        dev
    )?;
    let mut result: Vec<HashMap<String, String>> = Vec::new();
    for line in output.lines() {
        // parse key-value pairs
        result.push(split_lsblk_line(line));
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

pub fn get_blkdev_deps(device: &Path) -> Result<Vec<PathBuf>> {
    let deps = {
        let mut p = PathBuf::from("/sys/block");
        p.push(
            device
                .canonicalize()
                .chain_err(|| format!("canonicalizing {}", device.display()))?
                .file_name()
                .chain_err(|| format!("path {} has no filename", device.display()))?,
        );
        p.push("slaves");
        p
    };
    let mut ret: Vec<PathBuf> = Vec::new();
    for ent in read_dir(&deps).chain_err(|| format!("reading {}", &deps.display()))? {
        let ent = ent.chain_err(|| format!("reading {} entry", &deps.display()))?;
        ret.push(Path::new("/dev").join(ent.file_name()));
    }
    Ok(ret)
}

pub fn get_blkdev_deps_recursing(device: &Path) -> Result<Vec<PathBuf>> {
    let mut ret: Vec<PathBuf> = Vec::new();
    for dep in get_blkdev_deps(device)? {
        ret.extend(get_blkdev_deps_recursing(&dep)?);
        ret.push(dep);
    }
    Ok(ret)
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

/// Get the size of the GPT metadata at the start of the disk.
pub fn get_gpt_size(file: &mut (impl Read + Seek)) -> Result<u64> {
    let gpt = GPT::find_from(file).chain_err(|| "reading GPT")?;
    Ok(gpt.header.first_usable_lba * gpt.sector_size)
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

    runcmd!("udevadm", "settle")?;
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

// create unsafe ioctl wrappers
#[allow(clippy::missing_safety_doc)]
mod ioctl {
    use super::c_int;
    use nix::{ioctl_none, ioctl_read, ioctl_read_bad, request_code_none};
    ioctl_none!(blkrrpart, 0x12, 95);
    ioctl_read_bad!(blksszget, request_code_none!(0x12, 104), c_int);
    ioctl_read!(blkgetsize64, 0x12, 114, libc::size_t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use maplit::hashmap;
    use std::io::{copy, Read};
    use tempfile::tempfile;
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
            assert_eq!(
                detect_formatted_sector_size(&data),
                test.result,
                "{}",
                test.name
            );
        }
    }

    #[test]
    fn test_saved_partitions() {
        use PartitionFilter::*;

        let make_part = |i: u32, name: &str, start: u64, end: u64| {
            (
                i,
                GPTPartitionEntry {
                    partition_type_guid: make_guid("type"),
                    unique_parition_guid: make_guid(&start.to_string()),
                    starting_lba: start * 2048,
                    ending_lba: end * 2048 - 1,
                    attribute_bits: 0,
                    partition_name: name.into(),
                },
            )
        };

        let base_parts = vec![
            make_part(1, "one", 1, 1024),
            make_part(2, "two", 1024, 2048),
            make_part(3, "three", 2048, 3072),
            make_part(4, "four", 3072, 4096),
            make_part(5, "five", 4096, 5120),
            make_part(7, "seven", 5120, 6144),
            make_part(8, "eight", 6144, 7168),
            make_part(9, "nine", 7168, 8192),
            make_part(10, "", 8192, 8193),
            make_part(11, "", 8193, 8194),
        ];
        let image_parts = vec![
            make_part(1, "boot", 1, 384),
            make_part(2, "EFI-SYSTEM", 384, 512),
            make_part(4, "root", 1024, 2200),
        ];
        let merge_base_parts = vec![make_part(2, "unused", 500, 3500)];

        let index = |i| Some(NonZeroU32::new(i).unwrap());
        let label = |l| Label(glob::Pattern::new(l).unwrap());
        let tests = vec![
            // Partition range
            (
                vec![Index(index(5), None)],
                vec![
                    make_part(5, "five", 4096, 5120),
                    make_part(7, "seven", 5120, 6144),
                    make_part(8, "eight", 6144, 7168),
                    make_part(9, "nine", 7168, 8192),
                    make_part(10, "", 8192, 8193),
                    make_part(11, "", 8193, 8194),
                ],
                vec![
                    make_part(1, "boot", 1, 384),
                    make_part(2, "EFI-SYSTEM", 384, 512),
                    make_part(4, "root", 1024, 2200),
                    make_part(5, "five", 4096, 5120),
                    make_part(7, "seven", 5120, 6144),
                    make_part(8, "eight", 6144, 7168),
                    make_part(9, "nine", 7168, 8192),
                    make_part(10, "", 8192, 8193),
                    make_part(11, "", 8193, 8194),
                ],
            ),
            // Glob
            (
                vec![label("*i*")],
                vec![
                    make_part(5, "five", 4096, 5120),
                    make_part(8, "eight", 6144, 7168),
                    make_part(9, "nine", 7168, 8192),
                ],
                vec![
                    make_part(1, "boot", 1, 384),
                    make_part(2, "EFI-SYSTEM", 384, 512),
                    make_part(4, "root", 1024, 2200),
                    make_part(5, "five", 4096, 5120),
                    make_part(8, "eight", 6144, 7168),
                    make_part(9, "nine", 7168, 8192),
                ],
            ),
            // Missing label, single partition, irrelevant range
            (
                vec![
                    label("six"),
                    Index(index(7), index(7)),
                    Index(index(15), None),
                ],
                vec![make_part(7, "seven", 5120, 6144)],
                vec![
                    make_part(1, "boot", 1, 384),
                    make_part(2, "EFI-SYSTEM", 384, 512),
                    make_part(4, "root", 1024, 2200),
                    make_part(7, "seven", 5120, 6144),
                ],
            ),
            // Empty label match, multiple results
            (
                vec![label("")],
                vec![make_part(10, "", 8192, 8193), make_part(11, "", 8193, 8194)],
                vec![
                    make_part(1, "boot", 1, 384),
                    make_part(2, "EFI-SYSTEM", 384, 512),
                    make_part(4, "root", 1024, 2200),
                    make_part(10, "", 8192, 8193),
                    make_part(11, "", 8193, 8194),
                ],
            ),
            // Partition renumbering
            (
                vec![Index(index(4), None)],
                vec![
                    make_part(4, "four", 3072, 4096),
                    make_part(5, "five", 4096, 5120),
                    make_part(7, "seven", 5120, 6144),
                    make_part(8, "eight", 6144, 7168),
                    make_part(9, "nine", 7168, 8192),
                    make_part(10, "", 8192, 8193),
                    make_part(11, "", 8193, 8194),
                ],
                vec![
                    make_part(1, "boot", 1, 384),
                    make_part(2, "EFI-SYSTEM", 384, 512),
                    make_part(4, "root", 1024, 2200),
                    make_part(5, "four", 3072, 4096),
                    make_part(6, "five", 4096, 5120),
                    make_part(7, "seven", 5120, 6144),
                    make_part(8, "eight", 6144, 7168),
                    make_part(9, "nine", 7168, 8192),
                    make_part(10, "", 8192, 8193),
                    make_part(11, "", 8193, 8194),
                ],
            ),
            // No saved partitions
            (
                vec![Index(index(15), None)],
                vec![],
                merge_base_parts.clone(),
            ),
            // No filters
            (vec![], vec![], merge_base_parts.clone()),
        ];

        let mut base = make_disk(512, &base_parts);
        let mut image = make_disk(512, &image_parts);
        for (testnum, (filter, expected_blank, expected_image)) in tests.iter().enumerate() {
            // try overwriting on blank disk
            let saved = SavedPartitions::new_from_file(&mut base, 512, filter).unwrap();
            let mut disk = make_unformatted_disk();
            saved.overwrite(&mut disk).unwrap();
            assert!(disk_has_mbr(&mut disk), "test {}", testnum);
            let result = GPT::find_from(&mut disk).unwrap();
            assert_eq!(
                get_gpt_size(&mut disk).unwrap(),
                512 * result.header.first_usable_lba
            );
            assert_partitions_eq(expected_blank, &result, &format!("test {} blank", testnum));

            // try merging with image disk onto merge_base disk
            let mut disk = make_disk(512, &merge_base_parts);
            saved.merge(&mut image, &mut disk).unwrap();
            assert!(!disk_has_mbr(&mut disk), "test {}", testnum);
            let result = GPT::find_from(&mut disk).unwrap();
            assert_eq!(
                get_gpt_size(&mut disk).unwrap(),
                512 * result.header.first_usable_lba
            );
            assert_partitions_eq(expected_image, &result, &format!("test {} image", testnum));
            assert_eq!(
                saved.get_offset().unwrap(),
                match expected_blank.is_empty() {
                    true => None,
                    false => {
                        let (i, p) = &expected_blank[0];
                        Some((
                            p.starting_lba * 512,
                            format!("partition {} (\"{}\")", i, p.partition_name.as_str()),
                        ))
                    }
                },
                "test {}",
                testnum
            );
        }

        // test merging with unformatted initial disk
        let mut disk = make_unformatted_disk();
        let saved = SavedPartitions::new_from_file(&mut disk, 512, &vec![label("z")]).unwrap();
        let mut disk = make_disk(512, &merge_base_parts);
        saved.merge(&mut image, &mut disk).unwrap();
        let result = GPT::find_from(&mut disk).unwrap();
        assert_partitions_eq(&merge_base_parts, &result, "unformatted disk");

        // test overlapping partitions
        let saved =
            SavedPartitions::new_from_file(&mut base, 512, &vec![Index(index(1), index(1))])
                .unwrap();
        let mut disk = make_disk(512, &merge_base_parts);
        assert_eq!(
            saved.merge(&mut image, &mut disk).unwrap_err().to_string(),
            "disk partition 4 ('root') ends after start of saved partition 1 ('one')",
        );

        // test sector size mismatch
        let saved = SavedPartitions::new_from_file(&mut base, 512, &vec![label("*i*")]).unwrap();
        let mut image_4096 = make_disk(4096, &image_parts);
        assert_eq!(
            get_gpt_size(&mut image_4096).unwrap(),
            4096 * GPT::find_from(&mut image_4096)
                .unwrap()
                .header
                .first_usable_lba
        );
        let mut disk = make_disk(4096, &merge_base_parts);
        assert_eq!(
            saved
                .merge(&mut image_4096, &mut disk)
                .unwrap_err()
                .to_string(),
            "GPT sector size 4096 doesn't match expected 512"
        );

        // test copying invalid partitions
        let mut disk = make_unformatted_disk();
        let data = include_bytes!("../fixtures/gpt-512-duplicate-partition-guids.xz");
        copy(&mut XzDecoder::new(&data[..]), &mut disk).unwrap();
        assert_eq!(
            SavedPartitions::new_from_file(&mut disk, 512, &vec![label("*")])
                .unwrap_err()
                .to_string(),
            "failed dry run restoring saved partitions; input partition table may be invalid"
        );
    }

    // TODO: The partitions array assumes 512-byte sectors and we don't
    // scale the start/end values for 4096.  This doesn't matter right now
    // because the only use of 4096-byte sectors is in an error test.
    fn make_disk(sector_size: u64, partitions: &Vec<(u32, GPTPartitionEntry)>) -> File {
        let mut disk = make_unformatted_disk();
        // Make the disk just large enough for its partitions, then resize
        // it back up afterward.  This tests that we properly handle copying
        // saved partitions from the larger base disk into the smaller
        // install image.
        let len = if partitions.is_empty() {
            1024 * 1024
        } else {
            partitions[partitions.len() - 1].1.ending_lba * sector_size + 1024 * 1024
        };
        disk.set_len(len).unwrap();
        let mut gpt = GPT::new_from(&mut disk, sector_size, make_guid("disk")).unwrap();
        for (partnum, entry) in partitions {
            gpt[*partnum] = entry.clone();
        }
        gpt.write_into(&mut disk).unwrap();
        disk.set_len(10 * 1024 * 1024 * 1024).unwrap();
        disk
    }

    fn make_unformatted_disk() -> File {
        let disk = tempfile().unwrap();
        disk.set_len(10 * 1024 * 1024 * 1024).unwrap();
        disk
    }

    fn make_guid(seed: &str) -> [u8; 16] {
        let mut guid = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        for (i, b) in seed.as_bytes().iter().enumerate() {
            guid[i % guid.len()] ^= *b;
        }
        guid
    }

    fn disk_has_mbr(disk: &mut File) -> bool {
        let mut sig = [0u8; 2];
        disk.seek(SeekFrom::Start(510)).unwrap();
        disk.read_exact(&mut sig).unwrap();
        sig == [0x55, 0xaa]
    }

    fn assert_partitions_eq(expected: &Vec<(u32, GPTPartitionEntry)>, found: &GPT, message: &str) {
        // GPTPartitionEntry doesn't derive PartialEq.  Compare by hand.
        // first check that indexes are equal
        assert_eq!(
            expected.iter().map(|(i, _)| *i).collect::<Vec<u32>>(),
            found
                .iter()
                .filter(|(_, e)| e.is_used())
                .map(|(i, _)| i)
                .collect::<Vec<u32>>(),
            "{}",
            message
        );
        // check contents
        for (i, entry) in expected {
            assert_eq!(
                entry.partition_name.as_str(),
                found[*i].partition_name.as_str(),
                "{}, partition {}",
                message,
                i
            );
            assert_eq!(
                entry.partition_type_guid, found[*i].partition_type_guid,
                "{}, partition {}",
                message, i
            );
            assert_eq!(
                entry.unique_parition_guid, found[*i].unique_parition_guid,
                "{}, partition {}",
                message, i
            );
            assert_eq!(
                entry.starting_lba, found[*i].starting_lba,
                "{}, partition {}",
                message, i
            );
            assert_eq!(
                entry.ending_lba, found[*i].ending_lba,
                "{}, partition {}",
                message, i
            );
            assert_eq!(
                entry.attribute_bits, found[*i].attribute_bits,
                "{}, partition {}",
                message, i
            );
        }
    }
}
