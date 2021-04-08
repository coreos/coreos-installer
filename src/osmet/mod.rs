// Copyright 2020 Red Hat, Inc.
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

// Note the following terms are in use in this module:
// - the "unpacked" image refers to the fully blown up metal image (as it'd be read from a block
//   device)
// - extents for which we already have a mapping are "skipped"
// - the "packed" image refers to the metal image with all the extents for which we already have a
//   mapping skipped
// - the "xzpacked" image is the packed image compressed with xz

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::{File, OpenOptions};
use std::io::{copy, Seek, SeekFrom, Write};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use nix::mount;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use xz2::write::XzEncoder;

use crate::blockdev::*;
use crate::cmdline::*;
use crate::io::*;

mod fiemap;
mod file;
mod io_helpers;
mod unpacker;

use crate::osmet::fiemap::*;
use crate::osmet::file::*;
use crate::osmet::io_helpers::*;
use crate::osmet::unpacker::*;

// just re-export OsmetUnpacker
pub use crate::osmet::unpacker::OsmetUnpacker;

#[derive(Serialize, Deserialize, Debug)]
struct Mapping {
    extent: Extent,
    object: Sha256Digest,
}

#[derive(Serialize, Deserialize, Debug)]
struct OsmetPartition {
    start_offset: u64,
    end_offset: u64,
    mappings: Vec<Mapping>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Osmet {
    /// Partitions for which we've registered mappings.
    partitions: Vec<OsmetPartition>,
    /// Checksum of the final disk image. Used for unpacking verification.
    checksum: Sha256Digest,
    /// Size of the final disk image. Used for unpacking verification.
    size: u64,
}

pub fn osmet_fiemap(config: &OsmetFiemapConfig) -> Result<()> {
    eprintln!("{:?}", fiemap_path(config.file.as_str().as_ref())?);
    Ok(())
}

pub fn osmet_pack(config: &OsmetPackConfig) -> Result<()> {
    // First, mount the two main partitions we want to suck out data from: / and /boot. Note
    // MS_RDONLY; this also ensures that the partition isn't already mounted rw elsewhere.
    // Allow the root partition to be in a child holder device to allow for the RHCOS
    // crypto_LUKS partition.
    let disk = Disk::new(&config.device)?;
    let boot = disk.mount_partition_by_label("boot", false, mount::MsFlags::MS_RDONLY)?;
    let root = disk.mount_partition_by_label("root", true, mount::MsFlags::MS_RDONLY)?;

    // now, we do a first scan of the boot partition and pick up files over a certain size
    let boot_files = prescan_boot_partition(&boot)?;

    // generate the primary OSTree object <--> disk block mappings, and also try to match up boot
    // files with OSTree objects
    let (root_partition, mapped_boot_files) = scan_root_partition(&root, boot_files)?;

    let boot_partition = scan_boot_partition(&boot, mapped_boot_files)?;

    let partitions = vec![boot_partition, root_partition];

    // create a first tempfile to store the packed image
    eprintln!("Packing image");
    let (mut packed_image, size) =
        write_packed_image_to_file(Path::new(&config.device), &partitions, config.fast)?;

    // verify that re-packing will yield the expected checksum
    eprintln!("Verifying that repacked image matches digest");
    let (checksum, unpacked_size) =
        get_unpacked_image_digest(&mut packed_image, &partitions, &root)?;
    packed_image
        .seek(SeekFrom::Start(0))
        .context("seeking back to start of packed image")?;

    if unpacked_size != size {
        bail!(
            "unpacking test: got {} bytes but expected {}",
            unpacked_size,
            size
        );
    }

    let checksum_str = checksum_to_string(&checksum)?;
    if checksum_str != config.checksum {
        bail!(
            "unpacking test: got checksum {} but expected {}",
            checksum_str,
            &config.checksum
        );
    }

    let sector_size = get_sector_size_for_path(Path::new(&config.device))?.get();
    let header = OsmetFileHeader::new(sector_size, &config.description);

    // create final Osmet object to serialize
    let osmet = Osmet {
        partitions,
        checksum,
        size,
    };

    osmet_file_write(Path::new(&config.output), header, osmet, packed_image)?;
    eprintln!("Packing successful!");

    Ok(())
}

pub fn osmet_unpack(config: &OsmetUnpackConfig) -> Result<()> {
    // open output device for writing
    let mut dev = OpenOptions::new()
        .write(true)
        .open(Path::new(&config.device))
        .with_context(|| format!("opening {:?}", &config.device))?;

    if !dev
        .metadata()
        .with_context(|| format!("getting metadata for {:?}", &config.device))?
        .file_type()
        .is_block_device()
    {
        bail!("{:?} is not a block device", &config.device);
    }

    let mut unpacker = OsmetUnpacker::new(Path::new(&config.osmet), Path::new(&config.repo))?;
    copy(&mut unpacker, &mut dev)
        .with_context(|| format!("copying to block device {}", &config.device))?;

    Ok(())
}

pub fn find_matching_osmet_in_dir(
    osmet_dir: &Path,
    architecture: &str,
    sector_size: u32,
) -> Result<Option<(PathBuf, String)>> {
    for entry in WalkDir::new(osmet_dir).max_depth(1) {
        let entry = entry.with_context(|| format!("walking {:?}", osmet_dir))?;

        if !entry.file_type().is_file() {
            continue;
        }

        let header = osmet_file_read_header(entry.path())?;
        if header.os_architecture == architecture && header.sector_size == sector_size {
            return Ok(Some((entry.into_path(), header.os_description)));
        }
    }

    Ok(None)
}

fn scan_root_partition(
    root: &Mount,
    mut boot_files: HashMap<u64, PathBuf>,
) -> Result<(OsmetPartition, HashMap<PathBuf, Sha256Digest>)> {
    // query the trivial stuff first
    let (start_offset, end_offset) = root.get_partition_offsets()?;

    // we only hash boot files if there's a potential match with an OSTree object, so we keep a
    // cache to avoid recomputing it multiple times
    let mut cached_boot_files_digests: HashMap<u64, Sha256Digest> = HashMap::new();

    // boot files we were able to match up with OSTree objects
    let mut mapped_boot_files: HashMap<PathBuf, Sha256Digest> = HashMap::new();

    let objects_dir = root.mountpoint().join("ostree/repo/objects");

    let mut mappings: Vec<Mapping> = vec![];
    let mut mapped_file_count = 0;
    let mut empty_file_count = 0;
    for entry in WalkDir::new(objects_dir) {
        let entry = entry.context("walking objects/ dir")?;

        if !entry.file_type().is_file() {
            continue;
        }

        if entry.path().extension() != Some("file".as_ref()) {
            continue;
        }

        let extents = fiemap_path(entry.path().as_os_str())?;
        if extents.is_empty() {
            empty_file_count += 1;
            continue;
        }

        let object = object_path_to_checksum(entry.path())
            .with_context(|| format!("invalid object path {:?}", entry.path()))?;

        for extent in extents {
            mappings.push(Mapping {
                extent,
                object: object.clone(),
            });
        }

        // and check if this matches a boot file
        let len = entry
            .metadata()
            .with_context(|| format!("getting metadata for {:?}", entry.path()))?
            .len();
        if let Entry::Occupied(boot_entry) = boot_files.entry(len) {
            // we can't use Entry::or_insert_with() here because get_path_digest() is fallible
            let boot_file_digest = match cached_boot_files_digests.entry(len) {
                Entry::Vacant(e) => e.insert(get_path_digest(boot_entry.get())?),
                Entry::Occupied(e) => e.into_mut(),
            };
            if get_path_digest(entry.path())? == *boot_file_digest {
                mapped_boot_files.insert(boot_entry.remove(), object.clone());
            }
        }

        mapped_file_count += 1;
    }

    eprintln!(
        "Total OSTree objects scanned from /root: {} ({} mapped, {} empty)",
        mapped_file_count + empty_file_count,
        mapped_file_count,
        empty_file_count
    );

    eprintln!(
        "Total OSTree objects found in /boot: {}",
        mapped_boot_files.len()
    );

    canonicalize(&mut mappings);

    // would be cool to detect and report fragmented vs sparse files here too
    eprintln!("Total /root extents: {}", mappings.len());

    Ok((
        OsmetPartition {
            start_offset,
            end_offset,
            mappings,
        },
        mapped_boot_files,
    ))
}

fn prescan_boot_partition(boot: &Mount) -> Result<HashMap<u64, PathBuf>> {
    let mut files: HashMap<u64, PathBuf> = HashMap::new();

    for entry in WalkDir::new(boot.mountpoint()) {
        let entry = entry.context("walking /boot")?;

        if !entry.file_type().is_file() {
            continue;
        }

        let len = entry
            .metadata()
            .with_context(|| format!("getting metadata for {:?}", entry.path()))?
            .len();

        // The 1024 is chosen semi-arbitrarily; really, as long as the file is larger than the size
        // of one serialized `Mapping` (assuming no fragmentation), which is 56 bytes, we save
        // space. But we're not guaranteed an OSTree object match, and incur more overhead for
        // diminishing returns.
        if len > 1024 {
            files.entry(len).or_insert_with(|| entry.into_path());
        }
    }

    Ok(files)
}

fn scan_boot_partition(
    boot: &Mount,
    mut boot_files: HashMap<PathBuf, Sha256Digest>,
) -> Result<OsmetPartition> {
    // query the trivial stuff first
    let (start_offset, end_offset) = boot.get_partition_offsets()?;

    let mut mappings: Vec<Mapping> = vec![];

    for (path, object) in boot_files.drain() {
        for extent in fiemap_path(path.as_path().as_os_str())? {
            mappings.push(Mapping {
                extent,
                object: object.clone(),
            });
        }
    }

    canonicalize(&mut mappings);

    eprintln!("Total /boot extents: {}", mappings.len());

    Ok(OsmetPartition {
        start_offset,
        end_offset,
        mappings,
    })
}

/// Writes the disk image, with the extents for which we have mappings for skipped.
fn write_packed_image_to_file(
    block_device: &Path,
    partitions: &[OsmetPartition],
    fast: bool,
) -> Result<(File, u64)> {
    let mut xz_tmpf = XzEncoder::new(
        // ideally this would use O_TMPFILE, but since tempfile *needs* to create a named tempfile,
        // let's give it a descriptive name and extension
        tempfile::Builder::new()
            .prefix("coreos-installer-xzpacked")
            .suffix(".raw.xz")
            .tempfile()
            .context("allocating packed image tempfile")?
            // and here we delete it on disk so we just have an fd to it
            .into_file(),
        if fast { 0 } else { 9 },
    );

    let mut dev = OpenOptions::new()
        .read(true)
        .open(&block_device)
        .with_context(|| format!("opening {:?}", block_device))?;

    let total_bytes_skipped = write_packed_image(&mut dev, &mut xz_tmpf, partitions)?;

    xz_tmpf.try_finish().context("trying to finish xz stream")?;

    // sanity check that the number of bytes written + packed match up with block device size
    let blksize = get_block_device_size(&dev)
        .with_context(|| format!("querying block device size of {:?}", block_device))?;
    let total_bytes_written = xz_tmpf.total_in();
    if total_bytes_written + total_bytes_skipped != blksize.get() {
        bail!(
            "bytes written + bytes skipped != block device size: {} + {} vs {}",
            total_bytes_written,
            total_bytes_skipped,
            blksize
        );
    }

    eprintln!("Total bytes skipped: {}", total_bytes_skipped);
    eprintln!("Total bytes written: {}", total_bytes_written);
    eprintln!("Total bytes written (compressed): {}", xz_tmpf.total_out());

    let mut tmpf = xz_tmpf.finish().context("finishing xz stream")?;
    tmpf.seek(SeekFrom::Start(0))
        .context("seeking back to start of tempfile")?;

    Ok((tmpf, blksize.get()))
}

fn write_packed_image(
    dev: &mut File,
    w: &mut impl Write,
    partitions: &[OsmetPartition],
) -> Result<u64> {
    let mut buf = [0u8; 8192];

    let mut cursor: u64 = 0;
    let mut total_bytes_skipped: u64 = 0;
    for (i, partition) in partitions.iter().enumerate() {
        // first copy everything up to the start of the partition
        assert!(partition.start_offset >= cursor);
        copy_exactly_n(dev, w, partition.start_offset - cursor, &mut buf)?;
        total_bytes_skipped += write_packed_image_partition(dev, w, partition, &mut buf)
            .with_context(|| format!("packing partition {}", i))?;
        cursor = partition.end_offset;
    }

    // and finally write out the remainder of the disk
    copy(dev, w).context("copying remainder of disk")?;

    Ok(total_bytes_skipped)
}

fn write_packed_image_partition(
    dev: &mut File,
    w: &mut impl Write,
    partition: &OsmetPartition,
    buf: &mut [u8],
) -> Result<u64> {
    let mut total_bytes_skipped = 0;

    // and this is where the real fun begins!
    let mut cursor = partition.start_offset;
    for mapping in partition.mappings.iter() {
        // make offset relative to start of disk, not partition
        let extent_start = mapping.extent.physical + partition.start_offset;
        assert!(extent_start >= cursor);
        if cursor < extent_start {
            cursor += copy_exactly_n(dev, w, extent_start - cursor, buf)
                .context("while writing in between extents")?;
        }

        // this is the crucial space-saving step; we skip over the extent we have a mapping for
        dev.seek(SeekFrom::Current(mapping.extent.length.try_into().unwrap()))
            .with_context(|| format!("while skipping extent: {:?}", mapping.extent))?;
        total_bytes_skipped += mapping.extent.length;
        cursor += mapping.extent.length;
    }

    assert!(cursor <= partition.end_offset);

    // and now just transfer the rest of the partition
    copy_exactly_n(dev, w, partition.end_offset - cursor, buf)
        .context("copying remainder of partition")?;

    Ok(total_bytes_skipped)
}

fn canonicalize(mappings: &mut Vec<Mapping>) {
    if mappings.is_empty() {
        // technically nothing to do... but this is highly suspicious, so log it
        eprintln!("No mappings to canonicalize");
        return;
    }

    // first, we need the mappings sorted by physical offset, then length (longest first)
    mappings.sort_unstable_by(|a, b| {
        a.extent
            .physical
            .cmp(&b.extent.physical)
            .then_with(|| a.extent.length.cmp(&b.extent.length).reverse())
    });

    let mut clamped_mappings_count = 0;
    let mut mappings_to_delete: Vec<usize> = Vec::new();
    let mut last_mapping_physical_end = mappings[0].extent.physical + mappings[0].extent.length;
    for (i, mapping) in mappings.iter_mut().enumerate().skip(1) {
        let mapping_physical_end = mapping.extent.physical + mapping.extent.length;
        // first check if the extent is wholly-contained by the previous one
        if mapping_physical_end <= last_mapping_physical_end {
            mappings_to_delete.push(i);
        } else {
            // If the extent's start has an overlap with the previous one, clamp it. Optimally,
            // we'd want to favour larger extents since it's lower overhead when unpacking. But
            // really, OSTree objects normally have no reflinked extents between them (though this
            // would be an interesting question to investigate) -- this naive handling provides a
            // fallback so we don't just barf if we do hit that case.
            if mapping.extent.physical < last_mapping_physical_end {
                let n = last_mapping_physical_end - mapping.extent.physical;
                mapping.extent.logical += n;
                mapping.extent.physical += n;
                mapping.extent.length -= n;
                clamped_mappings_count += 1;
            }
            last_mapping_physical_end = mapping_physical_end;
        }
    }

    eprintln!("Duplicate extents dropped: {}", mappings_to_delete.len());
    eprintln!("Overlapping extents clamped: {}", clamped_mappings_count);

    for i in mappings_to_delete.into_iter().rev() {
        mappings.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::default::Default;

    #[test]
    fn test_canonicalize() {
        let mut mappings: Vec<Mapping> = Vec::new();
        mappings.push(Mapping {
            extent: Extent {
                logical: 100,
                physical: 100,
                length: 50,
            },
            object: Sha256Digest::default(),
        });
        canonicalize(&mut mappings);
        assert_eq!(mappings.len(), 1);
        assert_eq!(
            mappings[0].extent,
            Extent {
                logical: 100,
                physical: 100,
                length: 50
            }
        );

        mappings.push(Mapping {
            extent: Extent {
                logical: 100,
                physical: 100,
                length: 10,
            },
            object: Sha256Digest::default(),
        });
        mappings.push(Mapping {
            extent: Extent {
                logical: 110,
                physical: 110,
                length: 10,
            },
            object: Sha256Digest::default(),
        });
        mappings.push(Mapping {
            extent: Extent {
                logical: 140,
                physical: 140,
                length: 10,
            },
            object: Sha256Digest::default(),
        });
        canonicalize(&mut mappings);
        assert_eq!(mappings.len(), 1);
        assert_eq!(
            mappings[0].extent,
            Extent {
                logical: 100,
                physical: 100,
                length: 50
            }
        );

        mappings.push(Mapping {
            extent: Extent {
                logical: 140,
                physical: 140,
                length: 20,
            },
            object: Sha256Digest::default(),
        });
        mappings.push(Mapping {
            extent: Extent {
                logical: 150,
                physical: 150,
                length: 20,
            },
            object: Sha256Digest::default(),
        });
        canonicalize(&mut mappings);
        assert_eq!(mappings.len(), 3);
        assert_eq!(
            mappings[0].extent,
            Extent {
                logical: 100,
                physical: 100,
                length: 50
            }
        );
        assert_eq!(
            mappings[1].extent,
            Extent {
                logical: 150,
                physical: 150,
                length: 10
            }
        );
        assert_eq!(
            mappings[2].extent,
            Extent {
                logical: 160,
                physical: 160,
                length: 10
            }
        );
    }
}
