#!/bin/bash
# Check that every `install` subcommand long option is documented as a
# config file field

set -euo pipefail

rootdir="$(dirname $0)/.."
prog="${rootdir}/target/debug/coreos-installer"

echo "Checking config file docs..."

fail=0
docfile="customizing-install.md"
for opt in $("${prog}" install -h | (grep -Eo -- "--[a-zA-Z0-9_-]+" ||:)); do
    if [[ ${opt} == @(--help|--config-file) ]]; then
        continue
    fi
    if ! awk "/^##/ {active=0} /^#.* Config file format$/ {active=1} {if (active) {print}}" "${rootdir}/docs/${docfile}" | grep -q -- "^${opt#--}:"; then
        echo "${opt#--} not documented in ${docfile}"
        fail=1
    fi
done

if [ "${fail}" = 1 ]; then
    exit 1
fi
