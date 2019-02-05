# coreos-installer

`coreos-installer` is a script to install Fedora or Red Hat CoreOS to 
target disk. It can be invoked as a standalone script or during bootup
via a dracut module.

## MORE DOCS TO COME

## Kernel command line options for coreos-installer
* coreos.inst.install_dev - The block device on the system to install to
* coreos.inst.image_url - The url of the coreos image to install to this device
* coreos.inst.ignition_url - The url of the coreos ignition config (optional, enter
  coreos.inst.ignition_url=skip to not load an ignition config)
