#!/bin/bash
# Check help text maximum line length

set -euo pipefail

rootdir="$(dirname $0)/.."
help() {
    "${rootdir}/target/debug/coreos-installer" $* --help
}

hidden=
fail=0
total=0
checklen() {
    local length subcommand subcommands docfile docfile_display doc_section
    local longopts shortopts opt
    total=$((${total} + 1))
    echo "Checking coreos-installer $*..."

    length=$(help $* | wc -L)
    if [ "${length}" -gt 80 ] ; then
        echo "$* --help line length ${length} > 80"
        fail=1
    fi

    longopts=$(help $* | (grep -Eo -- "--[a-zA-Z0-9_-]+" ||:))
    shortopts=$(help $* | (grep -Eo -- "-[a-zA-Z0-9]," ||:) | tr -d ,)
    # Ignore subcommands with subcommands, and hidden subcommands
    if [ -z "${hidden}" ] && ! help $* | grep -q "SUBCOMMANDS:" ; then
        docfile="${rootdir}/docs/cmd/$1.md"
        docfile_display="$(realpath --relative-to=${rootdir} ${docfile})"
        doc_section=$(awk "/^# coreos-installer / {active=0} /^# coreos-installer $*$/ {active=1} {if (active) {print}}" "${docfile}")
        if [ -n "${doc_section}" ]; then
            for opt in ${longopts} ${shortopts}; do
                if [[ ${opt} == @(-h|--help) ]]; then
                    continue
                fi
                if ! echo "${doc_section}" | grep -qF -- "**${opt}**"; then
                    echo "$* ${opt} not documented in ${docfile_display}"
                    fail=1
                fi
            done
        else
            echo "$* not documented in ${docfile_display}"
            fail=1
        fi
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
