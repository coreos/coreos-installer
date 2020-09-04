---
layout: default
parent: Command line reference
nav_order: 4
---

# coreos-installer iso
{: .no_toc }

1. TOC
{:toc}

# coreos-installer iso ignition embed

## Description

Embed an Ignition config in an ISO image

## Usage

**coreos-installer iso ignition embed** [*options*] *ISO*

## Arguments

| **ISO** | ISO image |

## Options

| **--force**, **-f** | Overwrite an existing Ignition config |
| **--ignition-file**, **-i** *path* | Ignition config to embed [default: stdin] |
| **--output**, **-o** *path* | Write ISO to a new output file |

# coreos-installer iso ignition show

## Description

Show the embedded Ignition config from an ISO image

## Usage

**coreos-installer iso ignition show** *ISO*

## Arguments

| **ISO** | ISO image |

# coreos-installer iso ignition remove

## Description

Remove an existing embedded Ignition config from an ISO image

## Usage

**coreos-installer iso ignition remove** *ISO*

## Arguments

| **ISO** | ISO image |

## Options

| **--output**, **-o** *path* | Copy to a new file, instead of modifying in place |
