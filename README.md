# coreos-installer

`coreos-installer` is a script to install Fedora CoreOS (FCOS) or Red Hat 
Enterprise Linux CoreOS (RHCOS) to a target disk. It can be invoked as a 
standalone script or during bootup via a dracut module.

## Kernel command line options for coreos-installer running in the initramfs

* `coreos.inst=yes` - Instruct the installer to run
* `coreos.inst.install_dev` - The block device on the system to install to
* `coreos.inst.image_url` - The URL of the CoreOS image to install to this device
* `coreos.inst.ignition_url` - The URL of the CoreOS Ignition config (optional, enter
  `coreos.inst.ignition_url=skip` to not load an Ignition config)

## Using the installer on FCOS or RHCOS

This installer is incorporated into FCOS and RHCOS.
There are ISO and PXE images that can be downloaded that will allow for an
install to be performed on bare metal hardware. While ISO install is
supported, we recommend PXE if you environment supports it since it is
more friendly to automation.

### Installing from ISO

#### Grab an ISO image and bare metal image URL

For Fedora CoreOS you can download an ISO image from the
[download page](https://getfedora.org/coreos/download/).
You'll also need the URL of the bare metal raw image linked from that
page.

The ISO image can install in either legacy boot (BIOS) mode or in UEFI
mode. You can boot it in either mode, regardless of what mode the OS will
boot from once installed.

#### Perform the install

You can install on a bare metal machine by burning the ISO to
disk and booting it or using ISO redirection via a LOM interface.
Alternatively you can use a VM like so:

```
virt-install --name cdrom --ram 4500 --vcpus 2 --disk size=20 --accelerate --cdrom /path/to/fedora-coreos-30.20190801.0-installer.iso --network default
```

**NOTE**: To test UEFI boot add `--boot uefi` to the CLI call.

Alternatively you can use `qemu` directly.
Create a disk image which we can use as install target:

```
qemu-img create -f qcow2 fcos.qcow2 10G
```
Now, run the following qemu command:

```
qemu-system-x86_64 -accel kvm -name fcos -m 2048 -cpu host -smp 2 -netdev user,id=eth0,hostname=coreos -device virtio-net-pci,netdev=eth0 -drive file=/path/to/fcos.qcow2,format=qcow2  -cdrom /path/to/fedora-coreos-30.20190801.0-installer.iso
```

Once you have reached the boot menu, press `<TAB>` (isolinux) or
`e` (grub) to edit the kernel command line. Add the parameters to the
kernel command line telling it what you want it to do. For example:

- `coreos.inst.install_dev=sda`
- `coreos.inst.image_url=http://example.com/fedora-coreos-30.20190801.0-metal.raw.xz`
- `coreos.inst.ignition_url=http://example.com/config.ign`

Now press `<ENTER>` (isolinux) or `<CTRL-x>` (grub) to kick off the
install. The install will occur on tty2 and there are very few good
log statements or debug opportunities. The user experience here
needs work and is tracked in [#5](https://github.com/coreos/coreos-installer/issues/5).

The install will complete and eventually reboot the machine. After
reboot the machine will boot into the installed system and the
embedded Ignition config will run on first boot.

### Installing from PXE

#### Grab a PXE image and bare metal image

For Fedora CoreOS you can download a PXE kernel, initramfs image, and bare
metal image from the [download page](https://getfedora.org/coreos/download/).

The PXE image can install in either legacy boot (BIOS) mode or in UEFI
mode. You can boot it in either mode, regardless of what mode the OS will
boot from once installed.

#### Perform the install

Here is an example `pxelinux.cfg` for booting the installer images with
PXELINUX:

```
DEFAULT pxeboot
TIMEOUT 20
PROMPT 0
LABEL pxeboot
    KERNEL fedora-coreos-30.20190801.0-installer-kernel
    APPEND ip=dhcp rd.neednet=1 initrd=fedora-coreos-30.20190801.0-installer-initramfs.img console=tty0 console=ttyS0 coreos.inst=yes coreos.inst.install_dev=sda coreos.inst.image_url=http://192.168.1.101:8000/fedora-coreos-30.20190801.0-metal.raw.xz coreos.inst.ignition_url=http://192.168.1.101:8000/config.ign
IPAPPEND 2
```

If you don't know how to use this information to test a PXE install
you can start with something like
[these instructions](https://dustymabe.com/2019/01/04/easy-pxe-boot-testing-with-only-http-using-ipxe-and-libvirt/)
for testing out PXE installs via a local VM + Libvirt.

## Testing out the installer script by running it directly

Grab `coreos-installer` and execute it on an already booted system.

**NOTE** The installer writes directly to a block device (disk) and
         consumes the entire device. The device specified to the
         installer needs to be available and not currently in use. You
         cannot target a disk that is currently mounted.

The easiest way to access a disk that is not currently in use is to
boot up the coreos-installer ISO. If you boot the ISO and don't provide
any extra arguments you will be presented with a usage message and
then a prompt where you can execute the installer via the CLI:

```
/usr/libexec/coreos-installer -d sdd -i https://example.com/ignition.cfg -b https://example.com/fedora-coreos-30.20190801.0-metal.raw.xz
```

Afterwards you'll need to reboot the machine.

Alternatively, you can install coreos-installer on a desktop/laptop
machine and write out an image to a spare disk attached to the system.
This can be dangerous if you specify the wrong disk to the installer.

You'll want to make sure all of the 
[dependencies](https://github.com/coreos/coreos-installer/blob/master/dracut/30coreos-installer/module-setup.sh#L18)
are installed on your machine. If you are on Fedora you can install
the coreos-installer rpm (and all dependencies) using DNF via
`dnf install coreos-installer`. The path to the script will be
`/usr/libexec/coreos-installer`.

```
sudo /path/to/coreos-installer -d sdg -i https://example.com/ignition.cfg -b https://example.com/fedora-coreos-30.20190801.0-metal.raw.xz
```

Afterwards, remove the disk from the computer and insert it into and
boot the target machine where it is desired to run CoreOS.


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
version = 29
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
args+='coreos.inst.image_url=http://example.com/fedora-coreos-30.20190801.0-metal.raw.xz '
args+='coreos.inst.ignition_url=http://example.com/config.ign '
sudo virt-install --location ./ --extra-args="${args}" --network network=default --name installer --memory 2048 --disk size=10
```
