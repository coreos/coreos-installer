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

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub(super) struct Extent {
    pub logical: u64,
    pub physical: u64,
    pub length: u64,
}

pub(super) fn fiemap_path(path: &OsStr) -> Result<Vec<Extent>> {
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("opening {:?}", path))?;

    let fd = file.as_raw_fd();
    fiemap(fd).with_context(|| format!("mapping {:?}", path))
}

/// Returns the `Extent`s associated with the given file. Note that the physical offsets are
/// relative to the partition start on which the file resides.
fn fiemap(fd: RawFd) -> Result<Vec<Extent>> {
    let mut m = ffi::fiemap::new();
    let mut extents: Vec<Extent> = Vec::new();

    loop {
        m.fm_start = match extents.iter().last() {
            Some(extent) => extent.logical + extent.length,
            None => 0,
        };

        // just add FS_IOC_FIEMAP in the error msg; higher-level callers will provide more context
        unsafe { ffi::ioctl::fs_ioc_fiemap(fd, &mut m).context("ioctl(FS_IOC_FIEMAP)")? };
        if m.fm_mapped_extents == 0 {
            break;
        }

        let mut found_last = false;
        for extent in m.fm_extents.iter().take(m.fm_mapped_extents as usize) {
            // These three are not strictly errors; we could just ignore them and let them be part
            // of the packed image. Though let's error out for now, so that (1) we notice them and
            // investigate if they do occur, and (2) we don't end up in scenarios where lots of
            // extents fall in those buckets and we end up with hyperinflated osmet binaries.
            if extent.fe_flags & ffi::FIEMAP_EXTENT_NOT_ALIGNED > 0 {
                bail!("extent not aligned");
            } else if extent.fe_flags & ffi::FIEMAP_EXTENT_MERGED > 0 {
                bail!("file does not support extents");
            } else if extent.fe_flags & ffi::FIEMAP_EXTENT_ENCODED > 0 {
                bail!("extent encoded");
            // the ones below this, we do not expect to hit on a "dead" ro rootfs
            } else if extent.fe_flags & ffi::FIEMAP_EXTENT_DELALLOC > 0 {
                bail!("extent not allocated yet");
            } else if extent.fe_flags & ffi::FIEMAP_EXTENT_UNWRITTEN > 0 {
                bail!("extent preallocated");
            } else if extent.fe_flags & ffi::FIEMAP_EXTENT_UNKNOWN > 0 {
                bail!("extent inaccessible");
            }

            extents.push(Extent {
                logical: extent.fe_logical,
                physical: extent.fe_physical,
                length: extent.fe_length,
            });

            if extent.fe_flags & ffi::FIEMAP_EXTENT_LAST > 0 {
                found_last = true;
            }
        }

        if found_last {
            break;
        }
    }

    Ok(extents)
}

// nest it so it's private to us (ioctl! always declares as `pub`)
mod ffi {
    use std::mem::{size_of, zeroed};

    // The 32 here is somewhat arbitrary; it comes out to a bit less than a 2k buffer for the
    // whole struct. filefrag uses 16k on the stack, e4defrag uses ~220k on the heap. But we
    // can be much less hungry since we don't expect to operate on fragmented filesystems. That
    // way we can comfortably allocate on the stack.
    const EXTENT_COUNT: usize = 32;

    // This is a hack to get the size of the fiemap struct *without* the extents array. We could
    // use offset_of(fiemap, fm_extents) once that's available as a `const fn`.
    const FIEMAP_SIZE: u32 =
        (size_of::<fiemap>() as u32) - (size_of::<[fiemap_extent; EXTENT_COUNT]>() as u32);

    // https://github.com/torvalds/linux/blob/0a679e13ea30f85a1aef0669ee0c5a9fd7860b34/include/uapi/linux/fs.h#L208
    // We have to use _bad! here because we don't want the macro to use size_of::<fiemap> directly.
    #[allow(clippy::missing_safety_doc)]
    pub mod ioctl {
        use nix::{ioctl_readwrite_bad, request_code_readwrite};
        ioctl_readwrite_bad!(
            fs_ioc_fiemap,
            request_code_readwrite!(b'f', 11, super::FIEMAP_SIZE),
            super::fiemap
        );
    }

    // make this a submod so we can apply dead_code on the whole bunch
    #[allow(dead_code)]
    #[allow(clippy::unreadable_literal)]
    pub mod fiemap_extent_flags {
        pub const FIEMAP_EXTENT_LAST: u32 = 0x00000001; // Last extent in file.
        pub const FIEMAP_EXTENT_UNKNOWN: u32 = 0x00000002; // Data location unknown.
        pub const FIEMAP_EXTENT_DELALLOC: u32 = 0x00000004; // Location still pending. Sets EXTENT_UNKNOWN.
        pub const FIEMAP_EXTENT_ENCODED: u32 = 0x00000008; // Data can not be read while fs is unmounted
        pub const FIEMAP_EXTENT_DATA_ENCRYPTED: u32 = 0x00000080; // Data is encrypted by fs. Sets EXTENT_NO_BYPASS.
        pub const FIEMAP_EXTENT_NOT_ALIGNED: u32 = 0x00000100; // Extent offsets may not be block aligned.
        pub const FIEMAP_EXTENT_DATA_INLINE: u32 = 0x00000200; // Data mixed with metadata. Sets EXTENT_NOT_ALIGNED.
        pub const FIEMAP_EXTENT_DATA_TAIL: u32 = 0x00000400; // Multiple files in block. Sets EXTENT_NOT_ALIGNED.
        pub const FIEMAP_EXTENT_UNWRITTEN: u32 = 0x00000800; // Space allocated, but no data (i.e. zero).
        pub const FIEMAP_EXTENT_MERGED: u32 = 0x00001000; // File does not natively support extents. Result merged for efficiency.
        pub const FIEMAP_EXTENT_SHARED: u32 = 0x00002000; // Space shared with other files.
    }
    pub use fiemap_extent_flags::*;

    // https://github.com/torvalds/linux/blob/0a679e13ea30f85a1aef0669ee0c5a9fd7860b34/Documentation/filesystems/fiemap.txt#L15
    #[repr(C)]
    #[derive(Debug)]
    pub struct fiemap {
        pub fm_start: u64,  // logical offset (inclusive) at which to start mapping (in)
        pub fm_length: u64, // logical length of mapping which userspace cares about (in)
        pub fm_flags: u32,  // FIEMAP_FLAG_* flags for request (in/out)
        pub fm_mapped_extents: u32, // number of extents that were mapped (out)
        pub fm_extent_count: u32, // size of fm_extents array (in)
        pub fm_reserved: u32,
        pub fm_extents: [fiemap_extent; EXTENT_COUNT], // array of mapped extents (out)
    }

    // https://github.com/torvalds/linux/blob/0a679e13ea30f85a1aef0669ee0c5a9fd7860b34/Documentation/filesystems/fiemap.txt#L80
    #[repr(C)]
    #[derive(Debug)]
    pub struct fiemap_extent {
        pub fe_logical: u64,  // logical offset in bytes for the start of the extent
        pub fe_physical: u64, // physical offset in bytes for the start of the extent
        pub fe_length: u64,   // length in bytes for the extent
        pub fe_reserved64: [u64; 2],
        pub fe_flags: u32, // FIEMAP_EXTENT_* flags for this extent
        pub fe_reserved: [u32; 3],
    }

    impl fiemap {
        pub fn new() -> Self {
            let mut r: Self = unsafe { zeroed() };
            r.fm_extent_count = EXTENT_COUNT as u32;
            r.fm_length = std::u64::MAX;
            r
        }
    }
}
