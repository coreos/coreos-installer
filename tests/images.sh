#!/bin/bash

set -euo pipefail

export PATH
bindir="$(realpath -m "$(dirname "$0")/../target/${PROFILE:-debug}")"
if [[ -x "$bindir/coreos-installer" ]]; then
    PATH="$bindir:$PATH"
elif command -v coreos-installer >/dev/null; then
    echo "$bindir/coreos-installer not found"
    if [[ -n "${COREOS_INSTALLER_TEST_INSTALLED_BINARY:-}" ]]; then
        echo "COREOS_INSTALLER_TEST_INSTALLED_BINARY set; testing $(command -v coreos-installer)"
    else
        echo "Found $(command -v coreos-installer) but COREOS_INSTALLER_TEST_INSTALLED_BINARY not set; aborting"
        exit 1
    fi
else
    echo "coreos-installer not found.  Do you need to set PROFILE?"
    exit 1
fi
fixturesdir="$(realpath "$(dirname "$0")"/../fixtures)"

fixtures=(
    embed-areas-2020-09.iso.xz
    embed-areas-2021-01.iso.xz
    embed-areas-2021-09.iso.xz
    embed-areas-2021-12.iso.xz
    embed-areas-2022-02.iso.xz
    embed-areas-2022-09.iso.xz
    embed-areas-2023-03.x86_64.iso.xz
    embed-areas-2023-03.s390x.iso.xz
)

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
    "$(dirname "$0")/images/${cmd}" "$@"
}

call_for_fixtures() {
    local cmd="$1" first_fixture="$2"
    local fixture run=0
    for fixture in "${fixtures[@]}"; do
        if [ "$fixture" = "$first_fixture" ]; then
            run=1
        fi
        if [ $run = 1 ]; then
            call "$cmd" "${fixturesdir}/iso/${fixture}"
        fi
    done
    if [ $run = 0 ]; then
        echo "Unknown fixture ${first_fixture}; typo?"
        exit 1
    fi
}

if [ -n "${1:-}" ]; then
    # test with artifacts in cosa build dir
    basedir="$1"
    for f in "${basedir}"/*.iso; do
        if [[ ! -e "${f}" ]]; then
            echo "Couldn't find ISO image in ${basedir}"
            exit 1
        fi
    done
    call iso-ignition.sh "${basedir}"/*.iso
    call iso-network.sh "${basedir}"/*.iso
    call iso-kargs.sh "${basedir}"/*.iso
    call dev-show-iso.sh "${basedir}"/*.iso
    call iso-extract-minimal-iso.sh "${basedir}"/*.iso
    call iso-extract-pxe.sh "${basedir}"
    call customize.sh "${basedir}"
fi

# test historical layouts using fixtures
call_for_fixtures iso-ignition.sh embed-areas-2020-09.iso.xz
call_for_fixtures iso-network.sh embed-areas-2021-12.iso.xz
call_for_fixtures iso-kargs.sh embed-areas-2021-01.iso.xz
call_for_fixtures iso-extract-minimal-iso.sh embed-areas-2021-12.iso.xz
call unsupported.sh

# other image tests
call dev-initrd.sh

msg Success.
