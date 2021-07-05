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

# coreos-installer iso kargs modify

## Description

Modify kernel args in an ISO image

## Usage

**coreos-installer iso kargs modify** *ISO*

## Arguments

| **ISO** | ISO image |

## Options

| **--append**, **-a** *KARG...* | Kernel argument to append |
| **--delete**, **-d** *KARG...* | Kernel argument to delete |
| **--replace**, **-r** *KARG=OLDVAL=NEWVAL...* | Kernel argument to replace |
| **--output**, **-o** *path* | Write ISO to a new output file |

# coreos-installer iso kargs reset

## Description

Reset kernel args in an ISO image to defaults

## Usage

**coreos-installer iso kargs reset** *ISO*

## Arguments

| **ISO** | ISO image |

## Options

| **--output**, **-o** *path* | Write ISO to a new output file |

# coreos-installer iso kargs show

## Description

Show kernel args from an ISO image

## Usage

**coreos-installer iso kargs show** *ISO*

## Arguments

| **ISO** | ISO image |

## Options

| **--default**, **-d** | Show default kernel args |
