---
layout: default
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
