#!/bin/bash

set -euo pipefail

export PATH="$(realpath $(dirname $0)/../target/${PROFILE:-debug}):$PATH"
fixtures="$(realpath $(dirname $0)/../fixtures)"

msg() {
    cat <<EOF

[1;34m###############################################################################[0m
[1;34m$1[0m
[1;34m###############################################################################[0m
EOF
}

call() {
    msg "$*"
    local cmd="$1"
    shift
    "$(dirname $0)/images/${cmd}" "$@"
}

if [ -n "${1:-}" ]; then
    # test with artifacts in cosa build dir
    basedir="$1"
    if ! [ -e "${basedir}"/*.iso ] ;then
        echo "Couldn't find ISO image in ${basedir}"
        exit 1
    fi
    call iso-ignition.sh "${basedir}"/*.iso
    call iso-network.sh "${basedir}"/*.iso
    call iso-kargs.sh "${basedir}"/*.iso
    call dev-show-iso.sh "${basedir}"/*.iso
    call iso-extract-pxe.sh "${basedir}"
    call customize.sh "${basedir}"
fi

# test historical layouts using fixtures
call iso-ignition.sh ${fixtures}/iso/embed-areas-2020-09.iso.xz
call iso-ignition.sh ${fixtures}/iso/embed-areas-2021-01.iso.xz
call iso-ignition.sh ${fixtures}/iso/embed-areas-2021-09.iso.xz
call iso-ignition.sh ${fixtures}/iso/embed-areas-2021-12.iso.xz
call iso-network.sh ${fixtures}/iso/embed-areas-2021-12.iso.xz
call iso-kargs.sh ${fixtures}/iso/embed-areas-2021-01.iso.xz
call iso-kargs.sh ${fixtures}/iso/embed-areas-2021-09.iso.xz
call iso-kargs.sh ${fixtures}/iso/embed-areas-2021-12.iso.xz
call iso-kargs.sh ${fixtures}/iso/embed-areas-2022-02.iso.xz
call unsupported.sh

# other image tests
call dev-initrd.sh

msg Success.
