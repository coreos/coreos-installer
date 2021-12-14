---
nav_order: 5
---

# Customizing installation
{: .no_toc }

1. TOC
{:toc}

## Customizing coreos-installer invocation

coreos-installer can run automatically during boot of a CoreOS live image
(ISO or PXE) using either kernel command-line arguments or a config file.

[Kernel arguments](getting-started.md#kernel-command-line-options-for-coreos-installer-running-as-a-service)
are easier for simple cases, but not all coreos-installer parameters can be
specified that way.  For more complex cases, you can write one or more
config files to `/etc/coreos/installer.d`.  If any files exist in this
directory, coreos-installer will automatically run on boot, and will reboot
the live system after installation is complete.

To do this, specify an Ignition config to the live boot that runs the
installer.  This config is distinct from the Ignition config that governs
the installed system.

This is a sample Butane config that installs to `/dev/zda`:

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

All config files in the `installer.d` directory are evaluated in
alphabetical order, and any `coreos.inst` kernel command line arguments are
evaluated afterward.

## Hooking coreos-installer at boot time

When coreos-installer is run automatically from a CoreOS live image (ISO or
PXE), additional custom code can be run before or after the installer.  To
do this, specify an Ignition config to the live boot that runs the
installer.  This can be useful for automated hardware probing or interacting
with a provisioning system, e.g. to automatically select the target install
disk.

This is a sample Butane config with hooks that run both before and
after the installer:

```
variant: fcos
version: 1.1.0

storage:
  files:
    - path: /usr/local/bin/pre-install-hook
      mode: 0755
      contents:
        inline: |
          #!/bin/bash

          set -euo pipefail
          sleep 10
          echo "pre-hook"
    - path: /usr/local/bin/post-install-hook
      mode: 0755
      contents:
        inline: |
          #!/bin/bash

          set -euo pipefail
          sleep 10
          echo "post-hook"

systemd:
  units:
    - name: pre-install-hook.service
      enabled: true
      contents: |
        [Unit]
        Description=Run before install
        After=coreos-installer-pre.target
        Before=coreos-installer.service

        [Service]
        Type=oneshot
        ExecStart=/usr/local/bin/pre-install-hook

        [Install]
        RequiredBy=coreos-installer.service
    - name: post-install-hook.service
      enabled: true
      contents: |
        [Unit]
        Description=Run after install
        After=coreos-installer.service
        Before=coreos-installer.target

        [Service]
        Type=oneshot
        ExecStart=/usr/local/bin/post-install-hook

        [Install]
        RequiredBy=coreos-installer.target
```

Convert this Butane config to an Ignition config with:

```
butane < hooks.bu > hooks.ign
```

For live ISO booting, embed the resulting config in the live ISO:

```
coreos-installer iso ignition embed -i hooks.ign fedora-coreos-35.20211029.3.0-live.x86_64.iso
```

For live PXE booting, add Ignition first-boot arguments to the kernel argument
list:

```
coreos.inst.install_dev=/dev/sda coreos.inst.ignition_url=https://example.com/install-config.ign ignition.config.url=https://example.com/hooks.ign ignition.firstboot ignition.platform.id=metal
```
