---
parent: Command line reference
nav_order: 1
---

# coreos-installer install

## Description

Install Fedora CoreOS or RHEL CoreOS

## Usage

**coreos-installer install** [*options*] *device*

## Arguments

| **device** | Destination device |

## Options

| **--config-file**, **-c** *path* | YAML config file with install options |
| **--stream**, **-s** *name* | Fedora CoreOS stream |
| **--image-url**, **-u** *URL* | Manually specify the image URL |
| **--image-file**, **-f** *path* | Manually specify a local image file |
| **--ignition-file**, **-i** *path* | Embed an Ignition config from a file |
| **--ignition-url**, **-I** *URL* | Embed an Ignition config from a URL |
| **--ignition-hash** *digest* | Digest (type-value) of the Ignition config |
| **--platform**, **-p** *name* | Override the Ignition platform ID |
| **--append-karg** *arg1,arg2,...* | Append default kernel arg |
| **--delete-karg** *arg1,arg2,...* | Delete default kernel arg |
| **--copy-network**, **-n** | Copy network config from install environment |
| **--network-dir** *path* | For use with **-n** [default: /etc/NetworkManager/system-connections/] |
| **--save-partlabel** *lx,...* | Save partitions with this label glob |
| **--save-partindex** *id,...* | Save partitions with this number or range |
| **--offline** | Force offline installation |
| **--insecure** | Skip signature verification |
| **--insecure-ignition** | Allow Ignition URL without HTTPS or hash |
| **--stream-base-url** *URL* | Base URL for Fedora CoreOS stream metadata |
| **--architecture** *name* | Target CPU architecture [default: x86_64] |
| **--preserve-on-error** | Don't clear partition table on error |
| **--fetch-retries** *N* | Fetch retries, or string "infinite" |

## Config file format

Config files specified by `--config-file` are [YAML](https://yaml.org/) documents containing directives with the same names and semantics as command-line arguments.  Each specified config file is parsed in order, and other command-line arguments are parsed afterward.

All parameters are optional.

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
# Override the Ignition platform ID
platform: name
# Append default kernel arguments
append-karg: [arg1, arg2]
# Delete default kernel arguments
delete-karg: [arg1, arg2]
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
# Base URL for Fedora CoreOS stream metadata
stream-base-url: URL
# Target CPU architecture
architecture: name
# Don't clear partition table on error
preserve-on-error: true
# Fetch retries, or string "infinite"
fetch-retries: N
```
