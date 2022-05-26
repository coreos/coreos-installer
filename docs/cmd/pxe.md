---
parent: Command line reference
nav_order: 5
---

# coreos-installer pxe
{: .no_toc }

1. TOC
{:toc}

# coreos-installer pxe customize

```
Create a custom live PXE boot config

USAGE:
    coreos-installer pxe customize [OPTIONS] --output <path> <path>

ARGS:
    <path>
            CoreOS live initramfs image

OPTIONS:
        --dest-ignition <path>
            Ignition config fragment for dest sys

            Automatically run installer and merge the specified Ignition config into the config
            for the destination system.

        --dest-device <path>
            Install destination device

            Automatically run installer, installing to the specified destination device.  The
            resulting boot media will overwrite the destination device without confirmation.

        --dest-karg-append <arg>
            Destination kernel argument to append

            Automatically run installer, adding the specified kernel argument for every boot of
            the destination system.

        --dest-karg-delete <arg>
            Destination kernel argument to delete

            Automatically run installer, deleting the specified kernel argument for every boot
            of the destination system.

        --network-keyfile <path>
            NetworkManager keyfile for live & dest

            Configure networking using the specified NetworkManager keyfile. Network settings
            will be applied in the live environment, including when Ignition is run.  If
            installer is enabled via additional options, network settings will also be applied
            in the destination system, including when Ignition is run.

        --network-nmstate <path>
            Nmstate file for live & dest

            Configure networking using NetworkManager keyfiles generated from the specified
            Nmstate files. Network settings will be applied in the live environment, including
            when Ignition is run.  If installer is enabled via additional options, network
            settings will also be applied in the destination system, including when Ignition is
            run.

        --ignition-ca <path>
            Ignition PEM CA bundle for live & dest

            Specify additional TLS certificate authorities to be trusted by Ignition, in PEM
            format.  Authorities will be trusted by Ignition in the live environment and, if
            installer is enabled via additional options, in the destination system.

        --pre-install <path>
            Script to run before installation

            If installer is run at boot, run this script before installation. If the script
            fails, the live environment will stop at an emergency shell.

        --post-install <path>
            Script to run after installation

            If installer is run at boot, run this script after installation. If the script
            fails, the live environment will stop at an emergency shell.

        --installer-config <path>
            Installer config file

            Automatically run coreos-installer and apply the specified installer config file.
            Config files are applied in the order that they are specified.

        --live-ignition <path>
            Ignition config fragment for live env

            Merge the specified Ignition config into the config for the live environment.

    -o, --output <path>
            Output file

    -h, --help
            Print help information
```

# coreos-installer pxe ignition wrap

```
Wrap an Ignition config in an initrd image

USAGE:
    coreos-installer pxe ignition wrap [OPTIONS]

OPTIONS:
    -i, --ignition-file <path>    Ignition config to wrap [default: stdin]
    -o, --output <path>           Write to a file instead of stdout
    -h, --help                    Print help information
```

# coreos-installer pxe ignition unwrap

```
Show the wrapped Ignition config in an initrd image

USAGE:
    coreos-installer pxe ignition unwrap [initrd]

ARGS:
    <initrd>    initrd image [default: stdin]

OPTIONS:
    -h, --help    Print help information
```

# coreos-installer pxe network wrap

```
Wrap network settings in an initrd image

USAGE:
    coreos-installer pxe network wrap [OPTIONS] --keyfile <path>

OPTIONS:
    -k, --keyfile <path>    NetworkManager keyfile to embed
    -o, --output <path>     Write to a file instead of stdout
    -h, --help              Print help information
```

# coreos-installer pxe network unwrap

```
Extract wrapped network settings from an initrd image

USAGE:
    coreos-installer pxe network unwrap [OPTIONS] [initrd]

ARGS:
    <initrd>    initrd image [default: stdin]

OPTIONS:
    -C, --directory <path>    Extract to directory instead of stdout
    -h, --help                Print help information
```
