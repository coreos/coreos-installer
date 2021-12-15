---
parent: Command line reference
nav_order: 5
---

# coreos-installer pxe
{: .no_toc }

1. TOC
{:toc}

# coreos-installer pxe ignition wrap

```
Wrap an Ignition config in an initrd image

USAGE:
    coreos-installer pxe ignition wrap

OPTIONS:
    -i, --ignition-file <path>    Ignition config to wrap [default: stdin]
    -o, --output <path>           Write to a file instead of stdout
    -h, --help                    Prints help information
```

# coreos-installer pxe ignition unwrap

```
Show the wrapped Ignition config in an initrd image

USAGE:
    coreos-installer pxe ignition unwrap [initrd]

OPTIONS:
    -h, --help    Prints help information

ARGS:
    <initrd>    initrd image [default: stdin]
```
