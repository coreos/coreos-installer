---
nav_order: 8
---

# Release notes

## Upcoming coreos-installer 0.16.0 (unreleased)

Major changes:


Minor changes:

- Add release notes to documentation

Internal changes:


Packaging changes:

- Remove non-Linux dependencies from vendor archive


## coreos-installer 0.15.0 (2022-06-17)

Major changes:

- Add support for Secure Execution on s390x
- install: Perform platform-specific console configuration when `--platform` is specified and the OS supports it

Minor changes:

- Makefile: Don’t build during `make install`; install rdcore only if it was built

Internal changes:

- Add `zipl` command to reconfigure bootloader on s390x

Packaging changes:

- Update container to Fedora 36
- Require `nix` ≥ 0.24.0
- Remove all static libraries from vendor archive


## coreos-installer 0.14.0 (2022-04-27)

Major changes:

- Add aarch64 support to container image
- Add generated manpages

Minor changes:

- Silence error from reporting commands like `iso ignition show` and `pxe ignition unwrap` if output pipe is closed
- docs: Document `dest-device` config file field
- docs: Document `iso/pxe customize` commands
- docs: Drop documentation of pre/post installation hooks in favor of `customize` commands

Internal changes:

- bind-boot: Fix EFI vendor directory detection
- verify-unique-fs-label: use `blkid` instead of `lsblk` to make filesystem label querying more reliable
- Delete legacy aliases for `osmet` and `minimal-iso` pack commands used by coreos-assembler

Packaging changes:

- Require Rust ≥ 1.56.0
- Migrate from `structopt` to `clap` 3
- Drop `openat-ext` dependency
- Remove Windows binaries from vendor archive


## coreos-installer 0.13.1 (2022-02-13)

Major changes:

- Add Fedora 37 signing key; drop Fedora 34 signing key

Minor changes:

- install: Drop support for `COREOS_INSTALLER_NO_MOUNT_NAMESPACE`
- install: Eliminate partition table reread delay on busy block devices

Internal changes:

- Fix packing minimal ISO with empty files
- Move build-time packing commands to new `pack` subcommand
- Move developer-related commands to new `dev` subcommand
- Add `dev show initrd` and `dev extract initrd` subcommands
- verify-unique-fs-label: Add `--rereadpt` to reread partition tables first

Packaging changes:

- Disable LTO
- Disable debug symbols in container
- Require Rust ≥ 1.51.0


## coreos-installer 0.12.0 (2021-12-17)

Major changes:

- Add high-level [`iso customize`](https://coreos.github.io/coreos-installer/cmd/iso/#coreos-installer-iso-customize) and [`pxe customize`](https://coreos.github.io/coreos-installer/cmd/pxe/#coreos-installer-pxe-customize) subcommands for flexibly customizing a live image
- install: Add `--config-file` to specify install options via a YAML [config file](https://coreos.github.io/coreos-installer/customizing-install/#config-file-format)
- systemd: Automatically install if config files exist in `/etc/coreos/installer.d`
- Add [`iso network`](https://coreos.github.io/coreos-installer/cmd/iso/#coreos-installer-iso-network-embed) and [`pxe network`](https://coreos.github.io/coreos-installer/cmd/pxe/#coreos-installer-pxe-network-wrap) subcommands to embed NetworkManager configs in an ISO or wrap them in an initrd

Minor changes:

- Add [`iso reset`](https://coreos.github.io/coreos-installer/cmd/iso/#coreos-installer-iso-reset) subcommand to reset an ISO image to pristine state
- Differentiate `-h` and `--help`.  `--help` will produce longer-form documentation, similar to a man page.  Add long help to `install` and `iso/pxe customize`.
- pxe: Have [`ignition unwrap`](https://coreos.github.io/coreos-installer/cmd/pxe/#coreos-installer-pxe-ignition-unwrap) read from stdin if no filename specified
- pxe: Support [`ignition unwrap`](https://coreos.github.io/coreos-installer/cmd/pxe/#coreos-installer-pxe-ignition-unwrap) from concatenated initrds
- systemd: Print deprecation warning on `coreos.inst=yes`
- systemd: Print deprecation warning if `coreos.inst.install_dev` value omits `/dev`
- docs: Autogenerate [subcommand pages](https://coreos.github.io/coreos-installer/cmd/) from help text
- docs: Remove instructions for replacing `coreos-installer.service`, in favor of `installer.d` config files

Internal changes:

- Fix packing minimal ISO with hard-linked files
- bind-boot: Ignore ESPs not colocated with the boot filesystem
- rootmap: Properly handle linear RAID devices

Packaging changes:

- Add `base64`, `ignition-config`, `serde_with`, and `serde_yaml` dependencies
- Enable `structopt` `wrap_help` feature


## coreos-installer 0.11.0 (2021-11-18)

Major changes:

- Drop Fedora 33 signing key
- iso: Add `extract pxe` subcommand to extract PXE artifacts from ISO
- iso: Add `extract minimal-iso` subcommand to extract netboot ISO image from ISO

Minor changes:

- download: Ignore `--decompress` for artifacts that are meant to be used compressed
- download: Report the selected artifacts before starting download
- download/install: Avoid printing GPG verification result when we're ignoring it
- install: Report automatically selected OS, architecture, platform when downloading install image
- install: Report if multiple filesystems are labeled `boot`
- iso: Find Ignition embed area by directly parsing ISO filesystem
- iso: Find kargs embed areas by directly reading `kargs.json` from ISO, if available
- Add `-a` short option for `--architecture`
- Enable optimization for xz code in dev builds to speed up testing
- Fix build on s390x
- docs: Avoid using privileged container for `download` subcommand

Internal changes:

- Add support for packing minimal ISO
- rdcore: Add `bind-boot` subcommand to bind root and boot filesystems on first boot
- rdcore: Add `verify-unique-fs-label` subcommand to check if multiple filesystems share a label
- kargs: Add `--current` to do a dry run on the booted kargs
- osmet: Drop support for RHCOS unencrypted LUKS container

Packaging changes:

- Include debug symbols in release builds
- Add `bytes`, `structopt`, and `thiserror` dependencies
- Drop `clap` dependency
- Require `nix` &ge; 0.22
- dracut: Install `zipl_helper.device-mapper` on s390x
- Update container to Fedora 35
- Use Fedora build of liblzma in container


## coreos-installer 0.10.1 (2021-10-11)

Security fixes:

- Fix GPG signature check when decompressing gzipped images (GHSA-3r3g-g73x-g593, [CVE-2021-20319](https://access.redhat.com/security/cve/CVE-2021-20319))

Major changes:

- Add Fedora 36 signing key


## coreos-installer 0.10.0 (2021-08-05)

Major changes:

- install: Support IBM Z virtio DASD target devices
- install, download: Support retrying fetches with new `--fetch-retries` option

Minor changes:

- install: Restrict access permissions on `/boot/ignition` (GHSA-862g-9h5m-m3qv, [CVE-2021-3917](https://access.redhat.com/security/cve/CVE-2021-3917))
- install: Retry reading partition table on device mapper target devices
- systemd: Persist `coreos.force_persist_ip` kernel argument when installing with `coreos.inst.*`
- List subcommands of a command even without `-h`
- Mount filesystems in a separate mount namespace
- Refactor bootloader installation on s390x
- Enable optimization for gzip code in dev builds to speed up testing
- docs: Document `coreos-installer iso kargs` commands

Internal changes:

- kargs: Run `zipl` if necessary on s390x
- kargs: Don't fail `--create-if-changed` if the file already exists

Packaging changes:

- Require Rust &ge; 1.49.0
- Support OpenSSL 3.0


## coreos-installer 0.9.1 (2021-05-14)

Major changes:

- Add Fedora 35 signing key; drop Fedora 32 signing key

Minor changes:

- install: Fix block device path in error message when disk is busy
- install: Ignore corrupt GPT on target disk unless saving partitions

Internal changes:

- rootmap: Ignore multipath devices


## coreos-installer 0.9.0 (2021-04-08)

Major changes:

- iso: Support writing output file to stdout with `-o -`

Minor changes:

- blockdev: Fix RHEL `lsblk` ordering [bug](https://bugzilla.redhat.com/show_bug.cgi?id=1916502) by using `--nodeps` option
- blockdev: Strengthen device mapper path detection 

Internal changes:

- osmet: Drop support for `--real-rootdev` option 
- Add `--override-options` to `rdcore kargs` to make it easier to test kernel argument changes
- Optionally create a file if kernel arguments are modified
- Add declarative semantics for kernel argument modification

Packaging changes:

- Switch from `error-chain` to `anyhow` library 


## coreos-installer 0.8.0 (2021-01-12)

Major changes:

- Add `iso kargs` subcommand for modifying kernel arguments in live ISO images
- Support IBM Z FBA DASD target devices

Minor changes:

- Fix race condition causing `fdasd` failure when formatting ECKD DASD
- Fix sector size selection on unformatted ECKD DASD

Internal changes:

- rdcore: Add `kargs` subcommand for modifying BLS configurations

Packaging changes:

- Add `lazy_static` dependency
- Add `mbrman` and `rand` dependencies on s390x
- Require `regex` &ge; 1.4


## coreos-installer 0.7.2 (2020-10-22)

Major changes:

- Add Fedora 33 and 34 signing keys; drop Fedora 30 signing key

Minor changes:

- systemd: Start coreos-installer service after systemd-resolved

Packaging changes:

- Update container to Fedora 33


## coreos-installer 0.7.0 (2020-09-21)

Minor changes:

- iso: Use filesystem copy-on-write mechanism for `-o` output file if available
- install: Remember to update MBR from install image when restoring saved partitions
- install: Update size of protective MBR partition when restoring saved partitions
- install: Clear MBR boot code on install failure
- install: Revert insufficient s390x segfault avoidance change
- docs: Restructure for web publishing
- docs: Add initial command-line reference

Internal changes:

- rootmap: Configure LUKS root to wait on network if needed

Packaging changes:

- Require `openat-ext` &ge; 0.1.4


## coreos-installer 0.6.0 (2020-08-26)

Major changes:

- Add `pxe ignition` subcommands to generate or show an Ignition config wrapped in an appendable initrd
- iso: Move `iso` subcommands to `iso ignition` and deprecate the former
- iso: Rename `iso ignition embed -c`/`--config` to `-i`/`--ignition-file`

Minor changes:

- install: Fix kernel ignoring saved partitions after install failure
- install: Fix loss of saved partitions if original partition table is invalid
- install: Retain saved partitions in partition table at all times during install
- install: Clear partition table on failure by writing empty GPT rather than zeroes, except on DASD
- install: Reread kernel partition table after restoring partitions on failure
- install: Make `--preserve-on-error` saved partition stash file the same size as the target disk
- install: Properly activate first-boot kernel arguments on s390x
- install: Avoid segfault due to miscompilation in s390x release build
- iso: Compress Ignition config with XZ to increase capacity
- systemd: Suppress reboot after failure of a hook unit
- Document hooking install via an Ignition config

Internal changes:

- rootmap: Fix failure on unmodified rootfs
- rootmap: Inject `rootflags` kernel argument

Packaging changes:

- Require `gptman` &ge; 0.7


## coreos-installer 0.5.0 (2020-08-01)

Major changes:

- install: Add `--save-partlabel` and `--save-partindex` options to preserve specified partitions
- systemd: Add `coreos.inst.save_partlabel` and `coreos.inst.save_partindex` kargs

Minor changes:

- install: Fix installing to DASD via symlink
- systemd: Fix intermittent failure disabling MD-RAID and DM device activation
- systemd: Sequence `reboot`/`noreboot` services after `coreos-installer.target`
- Increase I/O block size when copying data, correctly

Internal changes:

- rdcore: Add `rootmap` subcommand to generate kargs for root device dependencies

Packaging changes:

- Don't build `rdcore` binary unless `rdcore` feature is enabled
- Add `glob` and `uuid` dependencies
- Drop `progress-streams` dependency
- Depend on `gptman` on all CPU architectures


## coreos-installer 0.4.0 (2020-07-24)

Minor changes:

- install: Support `sha256` hashes in `--ignition-hash`

Internal changes:

- rdcore: Add new program and Dracut module for internal use in the CoreOS initramfs
- rdcore: Add `stream-hash` subcommand for streaming verification of downloads
- osmet: Add `pack --fast` option for use in development builds
- osmet: Fix packing root filesystem wrapped in a `crypto_LUKS` container

Packaging changes:

- Allow `byte-unit` 3.x or 4.x
- Drop `sha2` dependency


## coreos-installer 0.3.0 (2020-07-13)

Major changes:

- install: Support remote Ignition configs with `--ignition-url`
- install: Support kernel argument modification with `--append-karg` and `--delete-karg`
- install: Support device-mapper target devices
- install: Support IBM Z DASD target devices

Minor changes:

- install: Deprecate `--firstboot-args`
- install: Report busy partitions if the target device is busy
- install: Correctly clear first MiB of target disk while copy is in progress
- install: Detect unreadable Ignition config before writing disk
- Increase I/O block size when copying data
- systemd: Correctly fail boot on systemd &ge; 245 if install fails
- systemd: Disable activation of MD-RAID and DM devices before install via new `coreos-installer-pre.target`

Packaging changes:

- Update container to Fedora 32
- Require bincode &ge; 1.3
- Require sha2 &ge; 0.9
- Require gptman on s390x
- Relax patchlevel version requirements on dependencies


## coreos-installer 0.2.1 (2020-05-30)

Major changes:

- Add Fedora 32 signing key; drop Fedora 30 signing key

Minor changes:

- Support creating offline image from RHCOS LUKS volume
- Add `coreos.inst` forwarding of `net.ifnames` and `net.naming-scheme` kargs


## coreos-installer 0.2.0 (2020-04-30)

Major changes:

- By default, install from an offline image shipped with the running system, if available
- When installing from a Fedora CoreOS stream, automatically select 4Kn image if needed
- Add `--copy-network` and `--network-dir` to copy network configs from the running system
- Add `--ignition-hash` to verify the hash of the specified Ignition config

Minor changes:

- Fix `coreos.inst` forwarding of repeated network kargs
- Fix `coreos.inst` forwarding of network kargs without a value
- Get block device path from `lsblk` rather than constructing it
- Stop overriding locked root account when launching emergency shell
- Redirect systemd generator's stdout to kmsg


## coreos-installer 0.1.3 (2020-03-20)

Major changes:

- Rename `--ignition` to `--ignition-file`.  `--ignition` is still accepted for compatibility.
- Don't discard disk contents before installing, so we don't delete data partitions when reprovisioning
- Add `--preserve-on-error` debug option to skip clearing partition table on error

Minor changes:

- Fail install early if image sector size doesn't match destination
- Ensure specified Ignition config exists before starting install
- Improve error message when mounting `/boot` if no partitions found
- Ignore HTTP errors on signature fetch if `--insecure` is specified
- Simplify download progress reporting if stderr is not a tty
- Retry when downloading `coreos.inst.ignition_url`
- Add upper-bound timeout to HTTP requests


## coreos-installer 0.1.2 (2020-01-08)

This is release v0.1.2 of coreos-installer.

Thanks to the following contributors for patches during this release:

- Benjamin Gilbert (2):
    - dracut: drop dracut modules
    - systemd: add scripts and systemd units for running at boot
- Colin Walters (1):
    - systemd: Add After=network-online.target
- Dusty Mabe (16):
    - systemd: service: fix calls to get commandline arg values
    - systemd: generator: rename cmdline_arg() to karg()
    - systemd: installer: indicate we want the network
    - systemd: reboot service: fix path to systemctl
    - systemd: generator: mv reboot flag file creation to generator
    - systemd: service: Make reboot service run after installer
    - systemd: services: log more to the console
    - systemd: use OnFailureJobMode=replace-irreversibly
    - systemd: add coreos-installer-noreboot.service
    - systemd: set SYSTEMD_SULOGIN_FORCE=1 for emergency.service
    - Remove coreos.inst.stream_base_url karg
    - systemd: make "network up" checking more robust
    - systemd: remove rudimentary network checking code
    - Cargo.toml: remove package.metadata.release.upload-doc option
    - Cargo.toml: use default tag prefix
    - Cargo.toml: replace deprecated option with new version
- Jonathan Lebon (1):
    - Add Makefile


## coreos-installer 0.1.1 (2019-12-18)

Changes:

- Improve error messages when rereading partition table
- Fix mounting boot device on CentOS 7.6
- verify: Explicitly trust imported keys
- verify: Switch to `always` trust model
- main: get rid of wildcard imports
- Packaging and release fixes
- docs: add technical details about iso-embedding
- README: Rewrite for Rust implementation
- docs: fix typo in release-checklist


## coreos-installer 0.1.0 (2019-11-08)

Changes:

- Initial release
