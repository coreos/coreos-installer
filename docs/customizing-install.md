---
layout: default
nav_order: 5
---

# Customizing installation
{: .no_toc }

1. TOC
{:toc}

## Hooking coreos-installer at boot time

When coreos-installer is run from a CoreOS live image (ISO or PXE) using
kernel command-line arguments, additional custom code can be run before or
after the installer.  To do this, specify an Ignition config to the live
boot that runs the installer.  This config is separate and distinct from the
Ignition config that governs the installed system.

This is a sample Fedora CoreOS Config with hooks that run both before and
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

Convert this FCC to an Ignition config with:

```
fcct < hooks.fcc > hooks.ign
```

For live ISO booting, embed the resulting config in the live ISO:

```
coreos-installer iso ignition embed -i hooks.ign fedora-coreos-32.20200715.3.0-live.x86_64.iso
```

For live PXE booting, add Ignition first-boot arguments to the kernel argument
list:

```
coreos.inst.install_dev=/dev/sda coreos.inst.ignition_url=https://example.com/install-config.ign ignition.config.url=https://example.com/hooks.ign ignition.firstboot ignition.platform.id=metal
```

## Custom coreos-installer invocation

It is also possible to invoke coreos-installer from a custom systemd unit,
instead of the default invocation that runs with the
`coreos.inst.install_dev` kernel argument.  This can be useful for
customizing the coreos-installer command-line arguments, e.g. to
automatically select the target install disk.

This is a sample Fedora CoreOS config to run coreos-installer:

```
variant: fcos
version: 1.1.0

systemd:
  units:
    - name: install.service
      enabled: true
      contents: |
        [Unit]
        Description=Run CoreOS Installer
        Requires=coreos-installer-pre.target
        After=coreos-installer-pre.target
        OnFailure=emergency.target
        OnFailureJobMode=replace-irreversibly

        # Can be removed if install doesn't reference remote resources with
        # --stream, --image-url, or --ignition-url
        After=network-online.target
        Wants=network-online.target

        [Service]
        Type=oneshot
        ExecStart=/usr/bin/coreos-installer install /dev/sda
        ExecStart=/usr/bin/systemctl --no-block reboot
        StandardOutput=kmsg+console
        StandardError=kmsg+console

        [Install]
        RequiredBy=default.target
```

Convert this FCC to an Ignition config with:

```
fcct < install.fcc > install.ign
```

For live ISO booting, embed the resulting config in the live ISO:

```
coreos-installer iso ignition embed -i install.ign fedora-coreos-32.20200715.3.0-live.x86_64.iso
```

For live PXE booting, use only the Ignition first-boot arguments in the kernel
argument list:

```
ignition.config.url=https://example.com/install.ign ignition.firstboot ignition.platform.id=metal
```
