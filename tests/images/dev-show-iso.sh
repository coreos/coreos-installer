#!/bin/bash
set -xeuo pipefail
PS4='${LINENO}: '

fatal() {
    echo "$@" >&2
    exit 1
}

iso=$1; shift
iso=$(realpath "${iso}")

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

coreos-installer dev show iso "${iso}" | tee inspect.json

# check that we found the descriptors
jq -e '.header.descriptors|length > 0' inspect.json

# specific descriptors
jq -e '.header.descriptors[]|select(.type == "primary")' inspect.json
jq -e '.header.descriptors[]|select(.type == "boot")' inspect.json

# check that we found some content
jq -e '.records|length > 0' inspect.json

# check that various fields are what we expect
jq -e '.header.descriptors[]|select(.type == "primary")|.system_id|contains("LINUX")' inspect.json
jq -e '.header.descriptors[]|select(.type == "primary")|.volume_id|contains("fedora-coreos")' inspect.json
jq -e '.header.descriptors[]|select(.type == "boot")|.boot_system_id|contains("EL TORITO")' inspect.json

# check that it found some various files and directories at various depths
jq -e '.records|index("EFI") >= 0' inspect.json
jq -e '.records|index("IMAGES/PXEBOOT") >= 0' inspect.json
jq -e '.records|index("IMAGES/PXEBOOT/ROOTFS.IMG") >= 0' inspect.json
jq -e '.records|index("ZIPL.PRM") >= 0' inspect.json

# Done
echo "Success."
