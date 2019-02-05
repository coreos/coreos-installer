# coreos-installer

`coreos-installer` is a script to install Fedora or Red Hat CoreOS to 
target disk. It can be invoked as a standalone script or during bootup
via a dracut module.

## MORE DOCS TO COME

## Kernel command line options for coreos-installer
* coreos.install_dev - The block device on the system to install to
* coreos.image_url - The url of the coreos image to install to this device
* coreos.ignition_url - The url of the coreos ignition config (optional, enter
  coreos.ignition_url=skip to not load an ignition config)
