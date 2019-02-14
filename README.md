# coreos-installer

`coreos-installer` is a script to install Fedora or Red Hat CoreOS to
target disk. It can be invoked as a standalone script or during bootup
via a dracut module.


## Kernel command line options for coreos-installer running in the initramfs
* coreos.inst=yes - Instruct the installer to run
* coreos.inst.install_dev - The block device on the system to install to
* coreos.inst.image_url - The url of the coreos image to install to this device
* coreos.inst.ignition_url - The url of the coreos ignition config (optional, enter
  coreos.inst.ignition_url=skip to not load an ignition config)

## Testing out the installer script

Grab /path/to/script and execute it on an already booted system.
You'll want to write to a disk that is not currently in use.

coreos-installer arg1 arg2 arg3 #todo teach installer to accept args

## Testing out the installer running in the initramfs (early boot)

You can build an initramfs with the installer/dracut module by cloning
this repo and building the initramfs locally like so:

```
git clone https://github.com/coreos/coreos-installer
cd coreos-installer
sudo dnf -y install dracut dracut-network
sudo dnf -y install $(grep inst_multiple dracut/30coreos-installer/module-setup.sh | sed 's|inst_multiple |\/usr\/bin\/|' | tr '\n' ' ')
sudo dnf -y install $(grep inst_multiple dracut/30coreos-installer/module-setup.sh | sed 's|inst_multiple||' | tr '\n' ' ')
sudo cp ./coreos-installer /usr/libexec/coreos-installer
sudo rsync -avh dracut/30coreos-installer /usr/lib/dracut/modules.d/
sudo dracut --kernel-cmdline="ip=dhcp rd.neednet=1" --add coreos-installer --no-hostonly -f ./initramfs.img --kver $(uname -r)
```

Take resulting initrd and a kernel and boot system with it
Provide `coreos.inst` arguments
```

You can then boot a system with that initrd and a kernel and see the
installer run. First we will grab the kernel.


```
cp /usr/lib/modules/$(uname -r)/vmlinuz ./vmlinuz
```

Then create a treeinfo file that can be used with virt-install:
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
