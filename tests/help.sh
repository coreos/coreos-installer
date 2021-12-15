#!/bin/bash
# Check help text maximum line length

set -euo pipefail

rootdir="$(dirname $0)/.."
help() {
    "${rootdir}/target/debug/coreos-installer" $* -h
}

hidden=
fail=0
total=0
checklen() {
    local length subcommand subcommands
    total=$((${total} + 1))
    echo "Checking coreos-installer $*..."

    length=$(help $* | wc -L)
    if [ "${length}" -gt 80 ] ; then
        echo "$* --help line length ${length} > 80"
        fail=1
    fi

    subcommands=$(help $* | awk 'BEGIN {subcommands=0} {if (subcommands) print $1} /SUBCOMMANDS:/ {subcommands=1}')
    for subcommand in ${subcommands}; do
        checklen $* ${subcommand}
    done
}

checklen
if [ ${total} -lt 2 ]; then
    echo "Detected no subcommands"
    fail=1
fi

# Hidden commands that users might invoke anyway (i.e. deprecated ones)
hidden=1
checklen iso embed
checklen iso show
checklen iso remove

if [ "${fail}" = 1 ]; then
    exit 1
fi
