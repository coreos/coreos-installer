# coreos-installer

`coreos-installer` is a script to install Fedora CoreOS (FCOS) or Red Hat 
Enterprise Linux CoreOS (RHCOS) to a target disk. It can be invoked as a 
standalone script or during bootup via a dracut module.


## Kernel command line options for coreos-installer running in the initramfs

* coreos.inst=yes - Instruct the installer to run
* coreos.inst.install_dev - The block device on the system to install to
* coreos.inst.image_url - The url of the coreos image to install to this device
* coreos.inst.ignition_url - The url of the coreos ignition config (optional, enter
  coreos.inst.ignition_url=skip to not load an ignition config)

## Using the installer on FCOS or RHCOS

This installer is incorporated into FCOS and RHCOS.
There are ISO images that can be downloaded that will allow for an
install to be performed on bare metal hardware either via ISO install
or via a PXE install. While ISO install is supported we certainly
recommend PXE if you environment supports it since it is more friendly
to automation.

#### Grab an ISO image and bare metal image

For Fedora CoreOS you can currently grab an ISO image from the output
of the current development pipeline located
[here](https://ci.centos.org/artifacts/fedora-coreos/prod/builds/latest/).

The ISO image can install in either legacy boot (BIOS) mode or in UEFI
mode. You'll have to make sure you download the appropriate related
image for the mode you'd like to use. It is a good idea to download
them before doing the install as the artifacts server that is being
used currently has very slow download speeds.

For example download:

- [fedora-coreos-29.731.iso](https://ci.centos.org/artifacts/fedora-coreos/prod/builds/latest/fedora-coreos-29.731.iso)

and one of the following two:

- [fedora-coreos-29.731-metal-bios.raw.gz](https://ci.centos.org/artifacts/fedora-coreos/prod/builds/latest/fedora-coreos-29.731-metal-bios.raw.gz)
- [fedora-coreos-29.731-metal-uefi.raw.gz](https://ci.centos.org/artifacts/fedora-coreos/prod/builds/latest/fedora-coreos-29.731-metal-uefi.raw.gz)

**NOTE** The artifacts output of the pipeline are development
         artifacts. The links above will quickly become
         broken because we prune builds. As we get closer to
         an official release we'll have stable links but for
         now you'll have to find your own links from the
         [latest directory](https://ci.centos.org/artifacts/fedora-coreos/prod/builds/latest/).

#### Test a PXE based install

Using the ISO images you can also do a PXE based install. You can
mount up the ISO images and use the `initramfs.img` and `vmlinuz`
for PXE boot. Here is an example `pxelinux.cfg` that I used to perform
a PXE boot:

```
DEFAULT pxeboot
TIMEOUT 20
PROMPT 0
LABEL pxeboot
    KERNEL fedora-coreos-29.731.iso/images/vmlinuz
    APPEND ip=dhcp rd.neednet=1 initrd=fedora-coreos-29.731.iso/images/initramfs.img console=tty0 console=ttyS0 coreos.inst=yes coreos.inst.install_dev=sda coreos.inst.image_url=http://192.168.1.101:8000/fedora-coreos-29.731-metal-bios.raw.gz coreos.inst.ignition_url=http://192.168.1.101:8000/config.ign
IPAPPEND 2
```

If you don't know how to use this information to test a PXE install
you can start with something like
[these instructions](https://dustymabe.com/2019/01/04/easy-pxe-boot-testing-with-only-http-using-ipxe-and-libvirt/)
for testing out PXE installs via a local VM + Libvirt.

#### Test an ISO based install

You can test an install on a bare metal machine by burning the ISO to
disk and booting it or using ISO redirection via a LOM interface.
Alternatively you can use a VM like so:

```
virt-install --name cdrom --ram 4500 --vcpus 2 --disk size=20 --accelerate --cdrom /path/to/fedora-coreos-29.731.iso --network default
```

**NOTE** To test UEFI boot add `--boot uefi` to the CLI call

Alternatively you can use `qemu` directly:

```
#TODO add qemu command here
```

One you have booted you will see a screen press `<TAB>` (isolinux) or
`e` (grub) to edit the kernel command line. Add the parameters to the
kernel command line telling it what you want it to do. For example:

- `coreos.inst.install_dev=sda`
- `coreos.inst.image_url=http://example.com/fedora-coreos-29.731-metal-bios.raw.gz`
- `coreos.inst.ignition_url=http://example.com/config.ign`

**NOTE** make sure to use a `metal-uefi` image if booting via UEFI

Now press `<ENTER>` (isolinux) or `<CTRL-x>` (grub) to kick off the
install. The install will occur on tty2 and there are very few good
log statements or debug opportunities. The user experience here
needs work and is tracked in [#5](https://github.com/coreos/coreos-installer/issues/5).

The install should progress and eventually reboot the machine. After
reboot the machine will boot into the installed system and the
embedded ignition config should run on first boot.



## Testing out the installer script by running it directly

**This does not work yet**

Grab `coreos-installer` and execute it on an already booted system.
You'll want to write to a disk that is not currently in use.

```
coreos-installer arg1 arg2 arg3
```

## Testing out the installer running in the initramfs (early boot)

You can build an initramfs with the installer/dracut module by cloning
this repo and building the initramfs locally like so:

```
git clone https://github.com/coreos/coreos-installer
cd coreos-installer
sudo dnf -y install dracut dracut-network
sudo dnf -y install $(grep inst_multiple dracut/30coreos-installer/module-setup.sh | sed 's|inst_multiple||' | tr '\n' ' ')
sudo cp ./coreos-installer /usr/libexec/coreos-installer
sudo rsync -avh dracut/30coreos-installer /usr/lib/dracut/modules.d/
sudo dracut --kernel-cmdline="ip=dhcp rd.neednet=1" --add coreos-installer --no-hostonly -f ./initramfs.img --kver $(uname -r)
```

You can then boot a system with that initrd and a kernel and see the
installer run. First we will grab the kernel.

```
cp /usr/lib/modules/$(uname -r)/vmlinuz ./vmlinuz
```

Then create a treeinfo file that can be used with `virt-install`:
This won't be necessary [in the future](https://bugzilla.redhat.com/show_bug.cgi?id=1677425).

```
cat <<'EOF' > .treeinfo
[general]
arch = x86_64
family = Fedora
platforms = x86_64
[images-x86_64]
initrd = initramfs.img
kernel = vmlinuz
EOF
```

Set our kernel arguments for the install and kick it off using
`virt-install`:

```
args='coreos.inst=yes '
args+='coreos.inst.install_dev=vda '
args+='coreos.inst.image_url=http://example.com/fedora-coreos-29.28-metal-bios.raw.gz '
args+='coreos.inst.ignition_url=http://example.com/config.ign '
sudo virt-install --location ./ --extra-args="${args}" --network network=default --name installer --memory 2048 --disk size=10
```
