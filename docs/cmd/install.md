---
parent: Command line reference
nav_order: 1
---

# coreos-installer install

```
Install Fedora CoreOS or RHEL CoreOS

USAGE:
    coreos-installer install [OPTIONS] <dest-device>

OPTIONS:
    -c, --config-file <path>...     YAML config file with install options
    -s, --stream <name>             Fedora CoreOS stream
    -u, --image-url <URL>           Manually specify the image URL
    -f, --image-file <path>         Manually specify a local image file
    -i, --ignition-file <path>      Embed an Ignition config from a file
    -I, --ignition-url <URL>        Embed an Ignition config from a URL
        --ignition-hash <digest>    Digest (type-value) of the Ignition config
    -a, --architecture <name>       Target CPU architecture [default: x86_64]
    -p, --platform <name>           Override the Ignition platform ID
        --append-karg <arg>...      Append default kernel arg
        --delete-karg <arg>...      Delete default kernel arg
    -n, --copy-network              Copy network config from install environment
        --network-dir <path>
            For use with -n [default: /etc/NetworkManager/system-connections/]

        --save-partlabel <lx>...    Save partitions with this label glob
        --save-partindex <id>...    Save partitions with this number or range
        --offline                   Force offline installation
        --insecure                  Skip signature verification
        --insecure-ignition         Allow Ignition URL without HTTPS or hash
        --stream-base-url <URL>     Base URL for Fedora CoreOS stream metadata
        --preserve-on-error         Don't clear partition table on error
        --fetch-retries <N>         Fetch retries, or "infinite" [default: 0]
    -h, --help                      Prints help information

ARGS:
    <dest-device>    Destination device
```
