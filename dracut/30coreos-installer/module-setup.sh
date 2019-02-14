#!/bin/bash
# module-setup for coreos 

# called by dracut
check() {
    require_binaries curl || return 1
    return 0 # default to install this module
}

# called by dracut
depends() {
    echo network url-lib
    return 0
}

# called by dracut
install() {
    inst_multiple chvt
    inst_multiple lsblk
    inst_multiple tee 
    inst_multiple gpg2
    inst_multiple curl
    inst_multiple wipefs 
    inst_multiple blockdev
    inst_multiple dd
    inst_multiple dialog
    inst_multiple dc
    inst_multiple awk
    inst_multiple pidof
    inst_multiple sha256sum
    inst_multiple zcat
    inst_simple /usr/libexec/coreos-installer
    inst_simple "$moddir/coreos-installer.service" "${systemdsystemunitdir}/coreos-installer.service"
    inst_hook cmdline 90 "$moddir/parse-coreos.sh"
    mkdir -p "${initdir}${systemdsystemconfdir}/initrd.target.wants"
    ln_r "${systemdsystemunitdir}/coreos-installer.service"\
        "${systemdsystemconfdir}/initrd.target.wants/coreos-installer.service"
}

