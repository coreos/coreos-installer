#!/bin/bash
# Check help text maximum line length

set -euo pipefail

fail=0
checklen() {
    local length
    length=$(target/debug/coreos-installer $* --help | wc -L)
    if [ "${length}" -gt 80 ] ; then
        echo "$* --help line length ${length} > 80"
        fail=1
    fi
}

checklen
checklen install
checklen download
checklen list-stream
checklen iso
checklen iso embed
checklen iso show
checklen iso remove
checklen iso ignition
checklen iso ignition embed
checklen iso ignition show
checklen iso ignition remove
checklen iso kargs modify
checklen iso kargs reset
checklen iso kargs show
checklen pxe
checklen pxe ignition
checklen pxe ignition wrap
checklen pxe ignition unwrap

if [ "${fail}" = 1 ]; then
    exit 1
fi
