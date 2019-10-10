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
    # bin
    inst_multiple /usr/bin/awk
    inst_multiple /usr/bin/curl
    inst_multiple /usr/bin/dc
    inst_multiple /usr/bin/dd
    inst_multiple /usr/bin/gpg2
    inst_multiple /usr/bin/grep
    inst_multiple /usr/bin/gzip
    inst_multiple /usr/bin/lsblk
    inst_multiple /usr/bin/ps
    inst_multiple /usr/bin/sha256sum
    inst_multiple /usr/bin/tail
    inst_multiple /usr/bin/tr
    inst_multiple /usr/bin/zcat
    inst_multiple /usr/bin/xzcat

    # sbin
    inst_multiple /usr/sbin/blockdev
    inst_multiple /usr/sbin/wipefs

    if [ "$arch" = "s390x" ]; then
        inst_multiple -o /usr/bin/wc
        inst_multiple -o /usr/sbin/zipl
        inst_multiple -o /usr/sbin/chreipl
        inst_multiple -o /usr/sbin/chzdev
        inst_multiple -o /usr/sbin/sfdisk
        inst_multiple -o /lib/s390-tools/stage3.bin
    fi

    inst_simple /usr/libexec/coreos-installer

    inst_simple "$moddir/coreos-installer-generator" \
        "$systemdutildir/system-generators/coreos-installer-generator"

    inst_simple "$moddir/coreos-installer.target" \
        "${systemdsystemunitdir}/coreos-installer.target"

    inst_simple "$moddir/coreos-installer.service" \
        "${systemdsystemunitdir}/coreos-installer.service"

    inst_hook cmdline 90 "$moddir/parse-coreos.sh"
}
