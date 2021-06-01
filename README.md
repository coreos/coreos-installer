# coreos-installer

[![Container image](https://quay.io/repository/coreos/coreos-installer/status)](https://quay.io/repository/coreos/coreos-installer)
[![crates.io](https://img.shields.io/crates/v/coreos-installer.svg)](https://crates.io/crates/coreos-installer)

coreos-installer is a program to assist with installing Fedora CoreOS
(FCOS) and Red Hat Enterprise Linux CoreOS (RHCOS). It can do the following:

* Install the operating system to a target disk, optionally customizing it
  with an Ignition config or first-boot kernel parameters
  ([`coreos-installer install`](docs/cmd/install.md))
* Download and verify an operating system image for various cloud,
  virtualization, or bare metal platforms ([`coreos-installer download`](docs/cmd/download.md))
* List Fedora CoreOS images available for download
  ([`coreos-installer list-stream`](docs/cmd/list-stream.md))
* Embed an Ignition config in a live ISO image to customize the running
  system that boots from it ([`coreos-installer iso ignition`](docs/cmd/iso.md))
* Wrap an Ignition config in an initrd image that can be appended to the
  live PXE initramfs to customize the running system that boots from it
  ([`coreos-installer pxe ignition`](docs/cmd/pxe.md))

The options available for each subcommand are available in the
[Command Line Reference](docs/cmd.md) or via the `--help` option.

Take a look at the [Getting Started Guide](docs/getting-started.md) for more
information regarding how to download and use `coreos-installer`.

## Contact

- Mailing list: [coreos@lists.fedoraproject.org](https://lists.fedoraproject.org/archives/list/coreos@lists.fedoraproject.org/)
- IRC: #[fedora-coreos](ircs://irc.libera.chat:6697/#fedora-coreos) on Libera.Chat
- Reporting bugs: [issues](https://github.com/coreos/coreos-installer/issues/new/choose)
