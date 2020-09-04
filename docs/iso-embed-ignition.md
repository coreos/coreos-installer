---
layout: default
parent: Development
nav_order: 1
---

# ISO-embedded Ignition configuration

CoreOS ISO images are typically used for live-booting machines directly from a read-only storage device (e.g. a CD-ROM or USB stick) on non-cloud platforms.

First-boot provisioning via Ignition in such environments is difficult, as there are no well-defined metadata endpoints, there may not be any hypervisor back-channels or writable disks, and manual entry of an Ignition URL on the kernel command line is not ergonomic.

For such reasons `coreos-installer` supports a special mode for ISO images, where an Ignition configuration file can be embedded as a user customization into a pristine image.
The resulting image can be then used to boot a live system which is provisioned with the given Ignition configuration.

## Technical details

The ISO-embedding mechanism works by modifying some raw data directly on the image.
The technical specifications are described below to help third-party logic.

CoreOS ISO images come with some reserved empty space, the "embed area", which can be used to inject the Ignition configuration.
The embed area is a block of zero-filled padding concatenated to the end of the initrd file.

The location and size of the embed area depends on each image, and can be read from a header in the disk's ISO9660 System Area (i.e. the first 32KiB of the disk).
The last 24 bytes of this 32 KiB area will contains the following fields, in this order and without special padding/alignment:

 * magic value (8 bytes): the ASCII value "coreiso+" as a marker to identify CoreOS ISO images.
 * location (8 bytes): little-endian unsigned 64-bits integer with the offset for the starting byte of the embed area.
 * length (8 bytes): little-endian unsigned 64-bits integer with the total length of the embed area.

The embed area is zeroed by default, meaning that the image is a pristine one without any user customization.

User customization is performed by writing a gzip-compressed `newc` cpio archive (i.e. the equivalent of a `.cpio.gz` file) to the embed area.
Such an archive must at least contain a regular file named `config.ign`, which can hold any custom Ignition configuration.

This archive is then detected and mounted in the initrd for Ignition consumption.
