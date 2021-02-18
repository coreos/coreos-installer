---
layout: default
parent: Development
nav_order: 2
---

# What is osmet?

CoreOS systems support booting into live PXE and ISO environments. These
work using a "rootfs initrd" which contains a squashfs of the actual
rootfs to boot into. This rootfs, like any OSTree-based system, contains
the system OSTree repo from which objects are hardlinked out to populate
the root filesystem tree.

In practice, the primary use case for these environments is to simply
install CoreOS to disk and reboot into the installed system. In the
past, installing meant fetching the raw metal image from a remote
location and writing it to disk. However, this is inefficient because
the majority of the data on that image come from the same
OSTree objects which are already present in the squashfs.

The osmet functionality is what now allows coreos-installer to re-use
these objects to install to disk, while still matching bit-for-bit the
metal image ("osmet" is a portmanteau of "OSTree" and "metal").

# How does osmet work?

At compose time (i.e. when we're creating metal images), osmet mounts
partitions from the raw metal image and uses the [FIEMAP] ioctl to build
a table of "OSTree object checksum -> disk offsets".

It then "packs" the raw image by going through it but skipping all the
chunks which correspond to mapped OSTree objects. The resulting packed
image then essentially only contains data like partition tables, the
BIOS boot partition, inode metadata, etc...

This packed image is passed through an xz filter and then bundled
together with the serialized OSTree object table into an "osmet" file.
coreos-assembler runs the packing twice: once for (regular) 512b sector
raw metal images, and once more for 4k sector images. Thus, we end up
with two osmet files.

Those files are then included as part of the rootfs initrd in the live
ISO and PXE environments alongside (not inside) the squashfs.

At install time (i.e. when users boot the live environment),
coreos-installer detects the osmet files present and uses the
appropriate one for the sector size of the target disk to recreate the
metal image to write to disk. The unpacking process is the inverse of
packing: it decompresses through xz, then with the deserialized lookup
table, it uses the OSTree objects from the mounted squashfs to fill in
the gaps which the packed object skipped over. Simultaneously, it
verifies the checksum of the written image to ensure that it exactly
matches the original.

All the osmet-related code is in `src/osmet/`. For more information, you
can also see the original PR here:

https://github.com/coreos/coreos-installer/pull/187

[FIEMAP]: https://www.kernel.org/doc/html/latest/filesystems/fiemap.html
