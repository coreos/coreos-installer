---
parent: Command line reference
nav_order: 4
---

# coreos-installer iso
{: .no_toc }

1. TOC
{:toc}

# coreos-installer iso customize

```
Customize a CoreOS live ISO image

USAGE:
    coreos-installer iso customize [OPTIONS] <ISO>

OPTIONS:
        --dest-ignition <path>...
            Ignition config fragment for dest sys

            Automatically run installer and merge the specified Ignition config into the config
            for the destination system.
        --dest-device <path>
            Install destination device

            Automatically run installer, installing to the specified destination device.  The
            resulting boot media will overwrite the destination device without confirmation.
        --dest-karg-append <arg>...
            Destination kernel argument to append

            Automatically run installer, adding the specified kernel argument for every boot of
            the destination system.
        --dest-karg-delete <arg>...
            Destination kernel argument to delete

            Automatically run installer, deleting the specified kernel argument for every boot
            of the destination system.
        --network-keyfile <path>...
            NetworkManager keyfile for live & dest

            Configure networking using the specified NetworkManager keyfile. Network settings
            will be applied in the live environment, including when Ignition is run.  If
            installer is enabled via additional options, network settings will also be applied
            in the destination system, including when Ignition is run.
        --pre-install <path>...
            Script to run before installation

            If installer is run at boot, run this script before installation. If the script
            fails, the live environment will stop at an emergency shell.
        --post-install <path>...
            Script to run after installation

            If installer is run at boot, run this script after installation. If the script
            fails, the live environment will stop at an emergency shell.
        --installer-config <path>...
            Installer config file

            Automatically run coreos-installer and apply the specified installer config file.
            Config files are applied in the order that they are specified.
        --live-ignition <path>...
            Ignition config fragment for live env

            Merge the specified Ignition config into the config for the live environment.
    -f, --force
            Overwrite existing customizations

    -o, --output <path>
            Write ISO to a new output file

    -h, --help
            Prints help information


ARGS:
    <ISO>
            ISO image

```

# coreos-installer iso ignition embed

```
Embed an Ignition config in an ISO image

USAGE:
    coreos-installer iso ignition embed [OPTIONS] <ISO>

OPTIONS:
    -f, --force                   Overwrite an existing Ignition config
    -i, --ignition-file <path>    Ignition config to embed [default: stdin]
    -o, --output <path>           Write ISO to a new output file
    -h, --help                    Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso ignition show

```
Show the embedded Ignition config from an ISO image

USAGE:
    coreos-installer iso ignition show <ISO>

OPTIONS:
    -h, --help    Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso ignition remove

```
Remove an existing embedded Ignition config from an ISO image

USAGE:
    coreos-installer iso ignition remove <ISO>

OPTIONS:
    -o, --output <path>    Write ISO to a new output file
    -h, --help             Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso network embed

```
Embed network settings in an ISO image

USAGE:
    coreos-installer iso network embed [OPTIONS] <ISO> --keyfile <path>...

OPTIONS:
    -k, --keyfile <path>...    NetworkManager keyfile to embed
    -f, --force                Overwrite existing network settings
    -o, --output <path>        Write ISO to a new output file
    -h, --help                 Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso network extract

```
Extract embedded network settings from an ISO image

USAGE:
    coreos-installer iso network extract <ISO>

OPTIONS:
    -C, --directory <path>    Extract to directory instead of stdout
    -h, --help                Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso network remove

```
Remove existing network settings from an ISO image

USAGE:
    coreos-installer iso network remove <ISO>

OPTIONS:
    -o, --output <path>    Write ISO to a new output file
    -h, --help             Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso kargs modify

```
Modify kernel args in an ISO image

USAGE:
    coreos-installer iso kargs modify <ISO>

OPTIONS:
    -a, --append <KARG>...                   Kernel argument to append
    -d, --delete <KARG>...                   Kernel argument to delete
    -r, --replace <KARG=OLDVAL=NEWVAL>...    Kernel argument to replace
    -o, --output <PATH>                      Write ISO to a new output file
    -h, --help                               Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso kargs reset

```
Reset kernel args in an ISO image to defaults

USAGE:
    coreos-installer iso kargs reset <ISO>

OPTIONS:
    -o, --output <PATH>    Write ISO to a new output file
    -h, --help             Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso kargs show

```
Show kernel args from an ISO image

USAGE:
    coreos-installer iso kargs show [OPTIONS] <ISO>

OPTIONS:
    -d, --default    Show default kernel args
    -h, --help       Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso extract pxe

```
Extract PXE files from an ISO image

USAGE:
    coreos-installer iso extract pxe <ISO>

OPTIONS:
    -o, --output-dir <PATH>    Output directory [default: .]
    -h, --help                 Prints help information

ARGS:
    <ISO>    ISO image
```

# coreos-installer iso extract minimal-iso

```
Extract a minimal ISO from a CoreOS live ISO image

USAGE:
    coreos-installer iso extract minimal-iso <ISO> [OUTPUT_ISO]

OPTIONS:
        --output-rootfs <PATH>    Extract rootfs image as well
        --rootfs-url <URL>        Inject rootfs URL karg into minimal ISO
    -h, --help                    Prints help information

ARGS:
    <ISO>           ISO image
    <OUTPUT_ISO>    Minimal ISO output file [default: -]
```

# coreos-installer iso reset

```
Restore a CoreOS live ISO image to default settings

USAGE:
    coreos-installer iso reset <ISO>

OPTIONS:
    -o, --output <path>    Write ISO to a new output file
    -h, --help             Prints help information

ARGS:
    <ISO>    ISO image
```
