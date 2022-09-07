---
nav_order: 5
---

# Customizing installation
{: .no_toc }

1. TOC
{:toc}

## Creating customized ISO and PXE images

The [`iso customize`](cmd/iso.md#coreos-installer-iso-customize) and
[`pxe customize`](cmd/pxe.md#coreos-installer-pxe-customize) commands can be
used to create customized ISO and PXE images with site-specific
configuration, including the ability to perform unattended installations.
This is the recommended method for automatically running coreos-installer at
boot.

For example:

```bash
# Create customized.iso which:
# - Automatically installs to /dev/sda
# - Provisions the installed system with config.ign
# - Uses network configuration from static-ip.nmconnection
# - Trusts HTTPS certificates signed by ca.pem
# - Runs post.sh after installing
coreos-installer iso customize \
    --dest-device /dev/sda \
    --dest-ignition config.ign \
    --network-keyfile static-ip.nmconnection \
    --ignition-ca ca.pem \
    --post-install post.sh \
    -o custom.iso input.iso
# Same, but create a customized PXE initramfs image
coreos-installer pxe customize \
    --dest-device /dev/sda \
    --dest-ignition config.ign \
    --network-keyfile static-ip.nmconnection \
    --ignition-ca ca.pem \
    --post-install post.sh \
    -o custom-initramfs.img input-initramfs.img
```

### Customize options

Available customizations include:

- Specifying an Ignition config to be applied to the installed system
  (`--dest-ignition`) or to the live environment where the installer runs
  (`--live-ignition`).
- Specifying the device to which the operating system will be installed
  (`--dest-device`).  If an ISO or PXE image has been customized with
  `--dest-device`, booting that image will automatically install to the
  specified disk and reboot the system.
- Specifying network configuration for both the installed system and the
  live environment via
  [NetworkManager keyfiles](https://developer.gnome.org/NetworkManager/stable/nm-settings-keyfile.html)
  (`--network-keyfile`).  The configuration is applied before Ignition runs,
  so this option is useful for specifying network settings that are needed
  for Ignition to fetch remote resources.
- Specifying HTTPS certificate authorities to be trusted by Ignition, in
  both the installed system and the live environment (`--ignition-ca`).
- Modifying kernel arguments of the installed system (`--dest-karg-append`,
  `--dest-karg-delete`) or the live ISO environment (`--live-karg-append`,
  `--live-karg-replace`, `--live-karg-delete`).  These options are useful if
  the machine will not boot at all without certain kernel arguments,
  preventing use of the Ignition `kernel_arguments` directives.  There are
  no `--live-karg` options for the PXE image; modify the PXE boot
  configuration instead.
- Running scripts before or after installation (`--pre-install`,
  `--post-install`).  For example, a pre-install script might run a
  container that performs hardware validation, or a post-install script
  might use IPMI to configure the machine to boot from the local disk
  instead of the network.  Pre-install scripts can change the options
  processed by coreos-installer, including the choice of destination device,
  by writing an installer config file to `/etc/coreos/installer.d` (see
  below).
- Specifying arbitrary options to `coreos-installer install` via an
  installer config file (see below).

All options except `--dest-device` can be specified multiple times.

## Customizing coreos-installer invocation

Alternatively, coreos-installer can be run automatically during boot of a
CoreOS live image (ISO or PXE) using either kernel command-line arguments
or a config file.

[Kernel arguments](getting-started.md#kernel-command-line-options-for-coreos-installer-running-as-a-service)
are easier for simple cases, but not all coreos-installer parameters can be
specified that way.  For more complex cases, you can write one or more
config files to `/etc/coreos/installer.d`.  If any files exist in this
directory, coreos-installer will automatically run on boot, and will reboot
the live system after installation is complete.

To do this, specify an Ignition config to the live boot that runs the
installer.  This config is distinct from the Ignition config that governs
the installed system.

All config files in the `installer.d` directory are evaluated in
alphabetical order, and any `coreos.inst` kernel command line arguments are
evaluated afterward.

### Config file format

Config files in `/etc/coreos/installer.d` (or specified by `--config-file`)
are [YAML](https://yaml.org/) documents containing directives with the same
names and semantics as command-line arguments.  Each specified config file
is parsed in order, and other command-line arguments are parsed afterward.

All parameters are optional.

<!-- begin example config -->
```yaml
# Fedora CoreOS stream
stream: name
# Manually specify the image URL
image-url: URL
# Manually specify a local image file
image-file: path
# Embed an Ignition config from a file
ignition-file: path
# Embed an Ignition config from a URL
ignition-url: URL
# Digest (type-value) of the Ignition config
ignition-hash: digest
# Target CPU architecture
architecture: name
# Override the Ignition platform ID
platform: name
# Append default kernel arguments
append-karg: [arg, arg]
# Delete default kernel arguments
delete-karg: [arg, arg]
# Copy network config from install environment
copy-network: true
# Source directory for copy-network
network-dir: path
# Save partitions with this label glob
save-partlabel: [glob, glob]
# Save partitions with this number or range
save-partindex: [id-or-range, id-or-range]
# Force offline installation
offline: true
# Skip signature verification
insecure: true
# Allow Ignition URL without HTTPS or hash
insecure-ignition: true
# Base URL for CoreOS stream metadata
stream-base-url: URL
# Don't clear partition table on error
preserve-on-error: true
# Fetch retries, or string "infinite"
fetch-retries: N
# Destination device
dest-device: path
```
<!-- end example config -->

### Example manual customization via `installer.d`

This is an example procedure for configuring an ISO or PXE installation to
`/dev/zda` using an installer config file.  The ISO procedure matches the
steps performed by `coreos-installer iso customize --dest-device`, and the
PXE procedure produces the same result as `coreos-installer pxe customize
--dest-device` via a slightly different path.

Write a Butane config that installs to `/dev/zda`:

```
variant: fcos
version: 1.4.0
storage:
  files:
    - path: /etc/coreos/installer.d/custom.yaml
      contents:
        inline: |
          dest-device: /dev/zda
```

Convert this Butane config to an Ignition config with:

```
butane < install.bu > install.ign
```

For live ISO booting, embed the resulting config in the live ISO:

```
coreos-installer iso ignition embed -i install.ign fedora-coreos-35.20211029.3.0-live.x86_64.iso
```

For live PXE booting, use only the Ignition first-boot arguments in the
kernel argument list:

```
ignition.config.url=https://example.com/install.ign ignition.firstboot ignition.platform.id=metal
```
