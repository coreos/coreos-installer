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
use gptman::GPT;
use std::fs::{read_to_string, File};
use std::io::{Cursor, Write};
use std::num::NonZeroU32;
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};

use crate::blockdev::{get_sector_size, udev_settle};
use crate::errors::*;
use crate::runcmd;
use crate::s390x::dasd::Range;
use crate::util::*;

const ECKD_DASD_BLOCKSIZE: u32 = 4096;

/// Returns expected sector size if this is a DASD we'll format later,
/// or None if the caller should use get_sector_size()
pub fn dasd_try_get_sector_size(device: &str) -> Result<Option<NonZeroU32>> {
    if is_formatted(device)? {
        Ok(None)
    } else {
        Ok(Some(NonZeroU32::new(ECKD_DASD_BLOCKSIZE).unwrap()))
    }
}

/// Generate partition table entries and byte ranges to copy
pub(crate) fn partition_ranges(header: &[u8], device: &mut File) -> Result<(Vec<Range>, Vec<String>)> {
    let bytes_per_block: u64 = get_sector_size(device)?.get().into();
    let blocks_per_track: u64 = get_sectors_per_track(device)?.get().into();

    let gpt = GPT::read_from(&mut Cursor::new(header), bytes_per_block)
        .chain_err(|| "reading GPT of source image")?;

    let mut ranges = Vec::new();
    let mut partitions = Vec::new();
    let mut start_track: u64 = 2; // the first 2 tracks of the ECKD DASD are reserved
    let entries = || gpt.iter().filter(|(_, pt)| pt.is_used());
    let (last_partition, _) = entries()
        .last()
        .chain_err(|| "source image has no partitions")?;

    for (i, pt) in entries() {
        let blocks = pt.ending_lba - pt.starting_lba + 1;
        let end_track = start_track + (blocks + blocks_per_track - 1) / blocks_per_track - 1;

        ranges.push(Range {
            in_offset: pt.starting_lba * bytes_per_block,
            out_offset: start_track * blocks_per_track * bytes_per_block,
            length: blocks * bytes_per_block,
        });

        if i == last_partition {
            partitions.push(format!("[{}, last, native]", start_track));
        } else {
            partitions.push(format!("[{}, {}, native]", start_track, end_track));
        };
        start_track = end_track + 1;
    }
    // partitions should be in offset order, but just to be sure
    ranges.sort_unstable_by_key(|r| r.in_offset);
    Ok((ranges, partitions))
}

/// Get disk bus id
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
fn bus_id(dasd: &str) -> Result<String> {
    let cmd = Command::new("lszdev")
        .arg("-n")
        .arg("-c")
        .arg("ID")
        .arg("--by-node")
        .arg(dasd)
        .stderr(Stdio::inherit())
        .output()
        .chain_err(|| format!("executing lszdev on {}", dasd))?;
    if !cmd.status.success() {
        bail!("lszdev on {} failed", dasd);
    }
    Ok(std::str::from_utf8(&cmd.stdout)
        .chain_err(|| "decoding lszdev output")?
        .trim_end()
        .to_string())
}

/// Check if disk is already formatted or not
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
fn is_formatted(dasd: &str) -> Result<bool> {
    let id = bus_id(dasd)?;
    let path = format!("/sys/bus/ccw/devices/{}/status", id);
    let contents = read_to_string(&path).chain_err(|| format!("reading {}", path))?;
    Ok(!contents.contains("unformatted"))
}

/// Check if disk is valid or not
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
pub(crate) fn is_invalid(dasd: &str) -> Result<bool> {
    let mut cmd = Command::new("fdasd");
    // we're looking for a hardcoded string in the output
    cmd.env("LC_ALL", "C").arg("-p").arg(dasd);
    let invalid = cmd_output(&mut cmd)?.contains("disk label block is invalid");
    // Older versions of `fdasd` open the device O_RDWR, which causes udev
    // to re-probe the device node.  This can cause the fdasd call in
    // default_format() to fail on 'Error while rereading partition table' or
    // 'Disk in use'.  To avoid this, wait for udev to settle.
    // https://bugzilla.redhat.com/1900699
    // Fixed by https://github.com/ibm-s390-tools/s390-tools/commit/3d74c53
    udev_settle()?;
    Ok(invalid)
}

/// Perform low-level format. This step is necessary before any further disk usage
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
pub(crate) fn low_level_format(dasd: &str) -> Result<()> {
    if is_formatted(dasd)? {
        eprintln!("Skipping low-level format for {}", dasd);
        return Ok(());
    }
    eprintln!("Performing low-level format for {}", dasd);
    runcmd!(
        "dasdfmt",
        "--blocksize",
        ECKD_DASD_BLOCKSIZE.to_string(),
        "--disk_layout",
        "cdl",
        "--mode",
        "full",
        "-y",
        "-p",
        dasd
    )?;
    udev_settle()?;
    Ok(())
}

/// Format disk and create partitions
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
/// * `partitions` - configuration strings
pub(crate) fn make_partitions(dasd: &str, partitions: &[String]) -> Result<()> {
    if partitions.len() > 3 {
        // fdasd silently ignores partitions after the first 3
        bail!("Can't create {} partitions, maximum 3", partitions.len());
    }
    let mut config = partitions.join("\n");
    config.push('\n');
    if try_format(dasd, &config).is_err() {
        default_format(dasd)?;
        try_format(dasd, &config)?;
    }
    Ok(())
}

/// If config-based format fails, then we have to perform
/// an auto-format on the whole disk
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
pub(crate) fn default_format(dasd: &str) -> Result<()> {
    eprintln!("Auto-partitioning {}", dasd);
    runcmd!("fdasd", "-a", "-s", dasd).chain_err(|| format!("auto-formatting {} failed", dasd))?;
    udev_settle()?;
    Ok(())
}

/// Format disk using a config file
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
/// * `config` - configuration file contents
fn try_format(dasd: &str, config: &str) -> Result<()> {
    eprintln!("Partitioning {}", dasd);
    let mut child = Command::new("fdasd")
        .arg("-s")
        .arg("--config")
        .arg("/dev/stdin")
        .arg(dasd)
        .stdin(Stdio::piped())
        .spawn()
        .chain_err(|| "failed to execute fdasd")?;
    child
        .stdin
        .as_mut()
        .chain_err(|| "couldn't open fdasd stdin")?
        .write_all(config.as_bytes())
        .chain_err(|| "couldn't write fdasd stdin")?;
    if !child
        .wait()
        .chain_err(|| "couldn't wait on fdasd")?
        .success()
    {
        bail!("couldn't format {} based on:\n{}", dasd, config);
    }
    udev_settle()?;
    Ok(())
}

/// Get the number of sectors per track of a block device.
fn get_sectors_per_track(file: &File) -> Result<NonZeroU32> {
    let fd = file.as_raw_fd();
    let mut geo: ioctl::hd_geometry = Default::default();
    match unsafe { ioctl::hdio_getgeo(fd, &mut geo) } {
        Ok(_) => {
            NonZeroU32::new(geo.sectors.into()).ok_or_else(|| "found sectors/track of zero".into())
        }
        Err(e) => Err(Error::with_chain(e, "getting disk geometry")),
    }
}

// create unsafe ioctl wrappers
mod ioctl {
    use nix::ioctl_read_bad;
    use std::os::raw::{c_uchar, c_ulong, c_ushort};

    #[repr(C)]
    #[derive(Debug, Default)]
    pub struct hd_geometry {
        pub heads: c_uchar,
        pub sectors: c_uchar,
        pub cylinders: c_ushort,
        pub start: c_ulong,
    }

    ioctl_read_bad!(hdio_getgeo, 0x0301, hd_geometry);
}
