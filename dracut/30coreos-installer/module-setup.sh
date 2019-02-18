#!/bin/bash
# module-setup for coreos-installer

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
    inst_multiple /usr/bin/chvt
    inst_multiple /usr/bin/lsblk
    inst_multiple /usr/bin/tee
    inst_multiple /usr/bin/gpg2
    inst_multiple /usr/bin/curl
    inst_multiple /usr/sbin/wipefs
    inst_multiple /usr/sbin/blockdev
    inst_multiple /usr/bin/dd
    inst_multiple /usr/bin/dialog
    inst_multiple /usr/bin/dc
    inst_multiple /usr/bin/awk
    inst_multiple /usr/bin/ps
    inst_multiple /usr/bin/sha256sum
    inst_multiple /usr/bin/zcat
    inst_simple /usr/libexec/coreos-installer
    inst_simple "$moddir/coreos-installer.service" "${systemdsystemunitdir}/coreos-installer.service"
    inst_hook cmdline 90 "$moddir/parse-coreos.sh"
    mkdir -p "${initdir}${systemdsystemconfdir}/initrd.target.wants"
    ln_r "${systemdsystemunitdir}/coreos-installer.service"\
        "${systemdsystemconfdir}/initrd.target.wants/coreos-installer.service"
}

