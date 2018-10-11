# coreos-dracut
dracut module to create bare metal installer initrds for coreos

## What is coreos-dracut?
coreos-dracut is a dracut module used to create initrds suitable for
headless bare metal installation of coreos systems

## How do you build an initrd with it?
Instructions for using dracut are packaged with the dracut software.

To create a coreos install initrd, you should do 3 things

1. Install this software in the dracut modules directory
2. Configure the dracut config file to include the kernel drivers you want and
   to include the coreos module
3. Run the dracut command to build an initrd against a given kernel

After preforming those steps, you can use the initrd and corresponding vmlinuz
kernel file to boot a bare metal system in any way you see fit.

## How do I actually do the install
Just boot the kernel/initrd via whatever method you see fit (pxe/uefi/boot
iso/etc).  The installer will start automatically.  By default the install is
interactive, prompting the user on the console for the image to install, the
device to install to, etc.  Interactive prompts can be bypassed if the
corresponding options are specified on the kernel command line

## Kernel command line options for coreos
* coreos.install_dev - The block device on the system to install to
* coreos.image_url - The url of the coreos image to install to this device
* coreos.ignition_url - The url of the coreos ignition config (optional, enter
  coreos.ignition_url=skip to not load an ignition config)
