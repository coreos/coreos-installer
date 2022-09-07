#!/bin/bash
# Regenerate example config file in customizing-install.md

set -euo pipefail

rootdir="$(dirname $0)/.."
prog="${rootdir}/target/${PROFILE:-debug}/coreos-installer"
path="${rootdir}/docs/customizing-install.md"

echo "Generating $(realpath --relative-to=${rootdir} ${path})..."
(
    awk '/^<!-- begin example config -->$/ {print; exit} {print}' "${path}"
    echo '```yaml'
    ${prog} pack example-config
    echo '```'
    awk '/^<!-- end example config -->$/ {p=1} {if (p) print}' "${path}"
) > "${path}.new"
mv "${path}.new" "${path}"
