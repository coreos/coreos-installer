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

use anyhow::{anyhow, bail, Context, Result};
use std::fs::{read_to_string, File};
use std::io::Write;
use std::num::NonZeroU32;
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};

use crate::blockdev::{get_sector_size, udev_settle};
use crate::runcmd;
use crate::s390x::dasd::{partitions_from_gpt_header, Range};
use crate::util::*;

const ECKD_DASD_BLOCKSIZE: u32 = 4096;

pub(crate) fn eckd_try_get_sector_size(dasd: &str) -> Result<Option<NonZeroU32>> {
    if is_formatted(dasd)? {
        Ok(None)
    } else {
        Ok(Some(NonZeroU32::new(ECKD_DASD_BLOCKSIZE).unwrap()))
    }
}

pub(crate) fn eckd_prepare(dasd: &str) -> Result<()> {
    low_level_format(dasd)?;
    if is_invalid(dasd)? {
        eprintln!("Disk {} is invalid, formatting", dasd);
        default_format(dasd)?
    }
    Ok(())
}

pub(crate) fn eckd_make_partitions(
    dasd: &str,
    device: &mut File,
    first_mb: &[u8],
) -> Result<Vec<Range>> {
    let (ranges, partitions) = partition_ranges(device, first_mb)?;
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
    Ok(ranges)
}

/// Generate partition table entries and byte ranges to copy
fn partition_ranges(device: &mut File, first_mb: &[u8]) -> Result<(Vec<Range>, Vec<String>)> {
    let bytes_per_block = get_sector_size(device)?.get() as u64;
    let blocks_per_track = get_sectors_per_track(device)?.get() as u64;
    let partitions = partitions_from_gpt_header(bytes_per_block, first_mb)?;
    let last = partitions.len() - 1;

    let mut start_track: u64 = 2; // the first 2 tracks of the ECKD DASD are reserved
    let mut ranges = Vec::new();
    let mut entries = Vec::new();

    for (idx, pt) in partitions.iter().enumerate() {
        let blocks = pt.ending_lba - pt.starting_lba + 1;
        ranges.push(Range {
            in_offset: pt.starting_lba * bytes_per_block,
            out_offset: start_track * blocks_per_track * bytes_per_block,
            length: blocks * bytes_per_block,
        });
        let end_track = start_track + (blocks + blocks_per_track - 1) / blocks_per_track - 1;

        if idx == last {
            entries.push(format!("[{}, last, native]", start_track));
        } else {
            entries.push(format!("[{}, {}, native]", start_track, end_track));
        };
        start_track = end_track + 1;
    }
    Ok((ranges, entries))
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
        .with_context(|| format!("executing lszdev on {}", dasd))?;
    if !cmd.status.success() {
        bail!("lszdev on {} failed", dasd);
    }
    Ok(std::str::from_utf8(&cmd.stdout)
        .context("decoding lszdev output")?
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
    let contents = read_to_string(&path).with_context(|| format!("reading {}", path))?;
    Ok(!contents.contains("unformatted"))
}

/// Check if disk is valid or not
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
fn is_invalid(dasd: &str) -> Result<bool> {
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
fn low_level_format(dasd: &str) -> Result<()> {
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

/// If config-based format fails, then we have to perform
/// an auto-format on the whole disk
///
/// # Arguments
/// * `dasd` - dasd device, i.e. smth like /dev/dasda
fn default_format(dasd: &str) -> Result<()> {
    eprintln!("Auto-partitioning {}", dasd);
    runcmd!("fdasd", "-a", "-s", dasd)
        .with_context(|| format!("auto-formatting {} failed", dasd))?;
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
        .context("failed to execute fdasd")?;
    child
        .stdin
        .as_mut()
        .context("couldn't open fdasd stdin")?
        .write_all(config.as_bytes())
        .context("couldn't write fdasd stdin")?;
    if !child.wait().context("couldn't wait on fdasd")?.success() {
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
        Ok(_) => NonZeroU32::new(geo.sectors.into())
            .ok_or_else(|| anyhow!("found sectors/track of zero")),
        Err(e) => Err(e).context("getting disk geometry"),
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
