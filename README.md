# coreos-installer

[![Build status](https://travis-ci.org/coreos/coreos-installer.svg?branch=master)](https://travis-ci.org/coreos/coreos-installer)
[![Container image](https://quay.io/repository/coreos/coreos-installer/status)](https://quay.io/repository/coreos/coreos-installer)
[![crates.io](https://img.shields.io/crates/v/coreos-installer.svg)](https://crates.io/crates/coreos-installer)

coreos-installer is a program to assist with installing Fedora CoreOS
(FCOS) and Red Hat Enterprise Linux CoreOS (RHCOS). It can do the following:

* Install the operating system to a target disk, optionally customizing it
  with an Ignition config or first-boot kernel parameters
  (`coreos-installer install`)
* Download and verify an operating system image for various cloud,
  virtualization, or bare metal platforms (`coreos-installer download`)
* List Fedora CoreOS images available for download
  (`coreos-installer list-stream`)
* Embed an Ignition config in a live ISO image to customize the running
  system that boots from it (`coreos-installer iso`)

The options available for each subcommand are available via the `--help`
option.  This documentation will focus on how to obtain and run
coreos-installer.

# On Fedora CoreOS or RHEL CoreOS

coreos-installer is included in Fedora CoreOS.  Just run
`coreos-installer` from the command line.  Fedora CoreOS provides
[live CD and network boot images](https://getfedora.org/coreos/download/)
you can run from RAM; you can use these to run coreos-installer to install
Fedora CoreOS or RHEL CoreOS to disk.

RHEL CoreOS also includes this, but note that versions &le; 4.4 contain
the `legacy` branch of this software.

# Run from a container

You can run coreos-installer from a container.  You'll need to bind-mount
`/dev` and `/run/udev`, as well as a data directory if you want to access
files in the host.  For example:

```sh
sudo podman run --pull=always --privileged --rm \
    -v /dev:/dev -v /run/udev:/run/udev -v .:/data \
    quay.io/coreos/coreos-installer:release \
    install /dev/vdb -s testing -i /data/config.ign
```

# Via a Fedora RPM

`coreos-installer` is packaged in Fedora:

```sh
sudo dnf install coreos-installer
```

Note in fact you can also do this inside a `podman run --privileged` type
container configured similarly to the above for the "pre-built container"
path, not necessarily the host's root filesystem.
See also [toolbox](https://github.com/containers/toolbox).

# Install with Cargo

You can also install just the coreos-installer binary with Rust's Cargo package manager:

```sh
cargo install coreos-installer
```

# Build and install from source tree

To build from the source tree:

```sh
make
```

To install the binary and systemd units to a target rootfs
(e.g. under a
[coreos-assembler](https://github.com/coreos/coreos-assembler)
workdir):

```sh
make install DESTDIR=/my/dest/dir
```

# Run from a live image using kernel command-line options

If you want a fully automated install, you can configure the Fedora CoreOS
live CD or netboot image to run coreos-installer and then reboot the system.
You do this by passing `coreos.inst.<arg>` arguments on the kernel command
line.

## Kernel command line options for coreos-installer running as a service

* `coreos.inst.install_dev` - The block device on the system to install to,
  such as `/dev/sda`.  Mandatory.
* `coreos.inst.stream` - Download and install the current release of
  Fedora CoreOS from the specified stream.  Optional; defaults to
  installing from local media if run from CoreOS live ISO or PXE media,
  and to `stable` on other systems.
* `coreos.inst.image_url` - Download and install the specified CoreOS image,
  overriding `coreos.inst.stream`.  Optional.
* `coreos.inst.ignition_url` - The URL of the Ignition config.  Optional.
  If missing, no Ignition config will be embedded, which is probably not
  what you want.
* `coreos.inst.platform_id` - The Ignition platform ID of the platform the
  CoreOS image is being installed on.  Optional; defaults to `metal`.
  Normally this should be specified only if installing inside a virtual
  machine.
* `coreos.inst.save_partlabel` - Comma-separated labels of partitions to
  preserve during the install.  Glob-style wildcards are permitted.  The
  specified partitions need not exist.  Optional.
* `coreos.inst.save_partindex` - Comma-separated indexes of partitions to
  preserve during the install.  Ranges (`m-n`) are permitted, and either `m`
  or `n` can be omitted.  The specified partitions need not exist.
  Optional.
* `coreos.inst.insecure` - Permit the OS image to be unsigned.  Optional.
* `coreos.inst.skip_reboot` - Don't reboot after installing.  Optional.

## Installing from ISO

[Download a Fedora CoreOS ISO image](https://getfedora.org/coreos/download/).
The ISO image can install in either legacy boot (BIOS) mode or in UEFI
mode. You can boot it in either mode, regardless of what mode the OS will
boot from once installed.

Burn the ISO to disk and boot it, or use ISO redirection via a LOM interface.
Alternatively you can use a VM like so:

```
virt-install --name cdrom --ram 4500 --vcpus 2 --disk size=20 --accelerate --cdrom /path/to/fedora-coreos-32.20200715.2.0-live.x86_64.iso --network default
```

Alternatively you can use `qemu` directly.  Create a disk image to use as
install target:

```
qemu-img create -f qcow2 fcos.qcow2 8G
```

Now, run the following qemu command:

```
qemu-system-x86_64 -accel kvm -name fcos -m 4500 -cpu host -smp 2 -netdev user,id=eth0,hostname=coreos -device virtio-net-pci,netdev=eth0 -drive file=/path/to/fcos.qcow2,format=qcow2  -cdrom /path/to/fedora-coreos-32.20200715.2.0-live.x86_64.iso
```

Once you have reached the boot menu, press `<TAB>` (isolinux) or
`e` (grub) to edit the kernel command line. Add the parameters to the
kernel command line telling it what you want it to do. For example:

- `coreos.inst.install_dev=/dev/sda`
- `coreos.inst.ignition_url=http://example.com/config.ign`

Now press `<ENTER>` (isolinux) or `<CTRL-x>` (grub) to kick off the
install.

The install will complete and eventually reboot the machine. After
reboot the machine will boot into the installed system and the
embedded Ignition config will run on first boot.

## Installing from PXE

[Download a Fedora CoreOS PXE kernel and initramfs image](https://getfedora.org/coreos/download/).
The PXE image can install in either legacy boot (BIOS) mode or in UEFI
mode. You can boot it in either mode, regardless of what mode the OS will
boot from once installed.

Here is an example `pxelinux.cfg` for booting the installer images with
PXELINUX:

```
DEFAULT pxeboot
TIMEOUT 20
PROMPT 0
LABEL pxeboot
    KERNEL fedora-coreos-32.20200715.2.0-live-kernel-x86_64
    APPEND initrd=fedora-coreos-32.20200715.2.0-live-initramfs.x86_64.img coreos.inst.install_dev=/dev/sda coreos.inst.ignition_url=http://192.168.1.101:8000/config.ign
IPAPPEND 2
```

If you don't know how to use this information to test a PXE install
you can start with something like
[these instructions](https://dustymabe.com/2019/01/04/easy-pxe-boot-testing-with-only-http-using-ipxe-and-libvirt/)
for testing out PXE installs via a local VM + libvirt.

# Build and test the installer for development

**NOTE:** The `install` subcommand writes directly to a block device (disk)
and consumes the entire device.  The device specified to the installer needs
to be available and not currently in use.  You cannot target a disk that is
currently mounted.

Build coreos-installer and use it to install a Fedora CoreOS `testing`
image to a partitionable loop device:

```sh
cargo build
truncate -s 8G image-file
sudo losetup -P /dev/loop0 image-file
sudo target/debug/coreos-installer install /dev/loop0 -s testing
```
