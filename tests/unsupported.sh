#!/bin/bash
set -xeuo pipefail
PS4='${LINENO}: '

fixtures="$(realpath $(dirname $0)/..)/fixtures"

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

unpack=(
    embed-areas-2020-09.iso
    embed-areas-2021-09.iso
    synthetic.iso
)
for f in "${unpack[@]}"; do
    xz -dc "${fixtures}/iso/${f}.xz" > "${f}"
done

try() {
    (coreos-installer "$@" 2>&1 ||:)
}

# iso customize feature handling is tested in customize.sh

# no Ignition embed area
echo '{"ignition": {"version": "3.0.0"}' |
    try iso ignition embed synthetic.iso |
    grep -q "Unrecognized CoreOS ISO image"

# no kargs embed area
try iso kargs modify -a foobar embed-areas-2020-09.iso |
     grep -q "No karg embed areas found"
try iso kargs modify -a foobar embed-areas-2020-09.iso -o out.iso |
     grep -q "No karg embed areas found"
try iso kargs modify -a foobar embed-areas-2020-09.iso -o - |
     grep -q "No karg embed areas found"
try iso kargs show embed-areas-2020-09.iso |
     grep -q "No karg embed areas found"
try iso kargs show --default embed-areas-2020-09.iso |
     grep -q "No karg embed areas found"
try iso kargs reset embed-areas-2020-09.iso -o - |
     grep -q "No karg embed areas found"
try iso kargs reset embed-areas-2020-09.iso -o out.iso |
     grep -q "No karg embed areas found"
try iso kargs reset embed-areas-2020-09.iso |
     grep -q "No karg embed areas found"

# no network settings support
try iso network embed -k "${fixtures}/customize/installer-test.nmconnection" \
    embed-areas-2021-09.iso |
    grep -q "does not support customizing network settings"

# no miniso support
try iso extract minimal-iso embed-areas-2021-09.iso minimal.iso |
    grep -q "does not support extracting a minimal ISO"

# no PXE images
try iso extract pxe synthetic.iso |
    grep -q "Unrecognized CoreOS ISO image"

# Done
echo "Success."
