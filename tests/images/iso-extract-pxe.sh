#!/bin/bash
set -xeuo pipefail
PS4='${LINENO}: '

fatal() {
    echo "$@" >&2
    exit 1
}

digest() {
    # Ignore filename
    sha256sum "${1:--}" | awk '{print $1}'
}

compare_digests() {
    local left=$1; shift
    local right=$1; shift
    local left_digest right_digest
    left_digest=$(digest "$left")
    right_digest=$(digest "$right")
    if [ "${left_digest}" != "${right_digest}" ]; then
        echo "$left: $left_digest" >&2
        echo "$right: $right_digest" >&2
        fatal "files do not match"
    fi
}

builddir=$1; shift
builddir=$(realpath "${builddir}")

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

# shellcheck disable=SC2086
iso=$(ls ${builddir}/*.iso)
coreos-installer iso extract pxe "${iso}"
base=$(basename "${iso}" .iso)

# check that the files are the same

# shellcheck disable=SC2086
compare_digests "${base}-vmlinuz" ${builddir}/*-kernel-*
# shellcheck disable=SC2086
compare_digests "${base}-initrd.img" ${builddir}/*-initramfs.*.img
# shellcheck disable=SC2086
compare_digests "${base}-rootfs.img" ${builddir}/*-rootfs.*.img

# Done
echo "Success."
