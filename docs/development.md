---
layout: default
has_children: true
nav_order: 9
---

# Development
{: .no_toc }

1. TOC
{:toc}

## Build and test the installer for development

**NOTE:** The `install` subcommand writes directly to a block device (disk) and
consumes the entire device. The device specified to the installer needs to be
available and not currently in use. You cannot target a disk that is currently
mounted.

Build coreos-installer and use it to install a Fedora CoreOS `testing` image to
a partitionable loop device:

```sh
cargo build
truncate -s 8G image-file
sudo losetup -P /dev/loop0 image-file
sudo target/debug/coreos-installer install /dev/loop0 -s testing
```

## Release process

Releases can be performed by [creating a new release ticket][new-release-ticket] and following the steps in the checklist there.

[new-release-ticket]: https://github.com/coreos/coreos-installer/issues/new?labels=release&template=release-checklist.md
