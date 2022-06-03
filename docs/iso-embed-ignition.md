---
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
The embed area is a block of padding stored at `/images/ignition.img` in the ISO image.
The bootloader is configured to load this file as an additional initrd image.
The embed area is zeroed by default, meaning that the image is a pristine one without any user customization.

User customization is performed by parsing the ISO9660 filesystem to determine the offset and length of the embed area file, and writing an xz-compressed `newc` cpio archive (i.e. the equivalent of a `.cpio.xz` file) directly into it.
Such an archive may contain a regular file named `config.ign`, which can hold any custom Ignition configuration.

This archive is then detected and unpacked in the initrd for Ignition consumption.
