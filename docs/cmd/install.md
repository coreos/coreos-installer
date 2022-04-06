---
parent: Command line reference
nav_order: 1
---

# coreos-installer install

```
Install Fedora CoreOS or RHEL CoreOS

USAGE:
    coreos-installer install [OPTIONS] [--] [DEST_DEVICE]

ARGS:
    <DEST_DEVICE>
            Destination device

            Path to the device node for the destination disk.  The beginning of the device will
            be overwritten without further confirmation.

OPTIONS:
    -c, --config-file <path>
            YAML config file with install options

            Load additional config options from the specified YAML config file. Later config
            files override earlier ones, and command-line options override config files.

            Config file keys are long option names without the leading "--". Values are strings
            for non-repeatable options, arrays of strings for repeatable options, and "true"
            for flags.  The destination device can be specified with the "dest-device" key.

    -s, --stream <name>
            Fedora CoreOS stream

            The name of the Fedora CoreOS stream to install, such as "stable", "testing", or
            "next".

    -u, --image-url <URL>
            Manually specify the image URL

    -f, --image-file <path>
            Manually specify a local image file

    -i, --ignition-file <path>
            Embed an Ignition config from a file

    -I, --ignition-url <URL>
            Embed an Ignition config from a URL

            Immediately fetch the Ignition config from the URL and embed it in the installed
            system.

        --ignition-hash <digest>
            Digest (type-value) of the Ignition config

            Verify that the Ignition config matches the specified digest, formatted as
            <type>-<hexvalue>.  <type> can be sha256 or sha512.

    -a, --architecture <name>
            Target CPU architecture

            Create an install disk for a different CPU architecture than the host.

            [default: x86_64]

    -p, --platform <name>
            Override the Ignition platform ID

            Install a system that will run on the specified cloud or virtualization platform,
            such as "vmware".

        --append-karg <arg>
            Append default kernel arg

            Add a kernel argument to the installed system.

        --delete-karg <arg>
            Delete default kernel arg

            Delete a default kernel argument from the installed system.

    -n, --copy-network
            Copy network config from install environment

            Copy NetworkManager keyfiles from the install environment to the installed system.

        --network-dir <path>
            Override NetworkManager keyfile dir for -n

            Specify the path to NetworkManager keyfiles to be copied with --copy-network.

            [default: /etc/NetworkManager/system-connections/]

        --save-partlabel <lx>
            Save partitions with this label glob

        --save-partindex <id>
            Save partitions with this number or range

    -h, --help
            Print help information

ADVANCED OPTIONS:
        --offline
            Force offline installation

        --insecure
            Skip signature verification

        --insecure-ignition
            Allow Ignition URL without HTTPS or hash

        --stream-base-url <URL>
            Base URL for CoreOS stream metadata

            Override the base URL for fetching CoreOS stream metadata. The default is
            "https://builds.coreos.fedoraproject.org/streams/".

        --preserve-on-error
            Don't clear partition table on error

            If installation fails, coreos-installer normally clears the destination's partition
            table to prevent booting from invalid boot media.  Skip clearing the partition
            table as a debugging aid.

        --fetch-retries <N>
            Fetch retries, or "infinite"

            Number of times to retry network fetches, or the string "infinite" to retry
            indefinitely.

            [default: 0]
```
