#!/bin/bash

install() {
    local _arch=${DRACUT_ARCH:-$(uname -m)}

    inst_simple "$moddir/rdcore" "/usr/bin/rdcore"

    # `rdcore kargs` calls `zipl` on s390x
    if [[ "$_arch" == "s390x" ]]; then
        inst_multiple zipl
        inst /lib/s390-tools/stage3.bin
        inst /lib/s390-tools/zipl_helper.device-mapper
    fi
}
