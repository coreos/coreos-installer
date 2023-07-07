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

grepq() {
    # Emulate grep -q without actually using it, to avoid propagating write
    # errors to the writer after a match, which would cause problems with
    # -o pipefail
    grep "$@" > /dev/null
}

iso=$1; shift
iso=$(realpath "${iso}")

if [[ "${iso}" == *2023-03.s390x.* ]]; then
    echo "Skipped; minimal ISO not supported on s390x before July 2023"
    exit 0
fi

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

if [ "${iso%.xz}" != "${iso}" ]; then
    xz -dc "${iso}" > test.iso
else
    cp --reflink=auto "${iso}" "test.iso"
fi
iso=test.iso

# Get expected hash of rootfs.  iso-extract-pxe.sh tests this against
# ground truth.
coreos-installer iso extract pxe -o pxe "${iso}"
rootfs_hash=$(digest pxe/*rootfs*)

# Get expected hash of output.  We don't have independent ground truth,
# but we assume the extractor checks the recorded hash of the output, and
# spot-check that assumption below.
coreos-installer iso extract minimal-iso "${iso}" out
miniso_hash=$(digest out)

# Verify that input has coreos.liveiso karg, and output doesn't
coreos-installer iso kargs show "${iso}" | grepq coreos.liveiso
if coreos-installer iso kargs show out | grepq coreos.liveiso; then
    fatal "Output contains coreos.liveiso karg"
fi

# Check output to stdout
coreos-installer iso extract minimal-iso "${iso}" > out
hash=$(digest out)
if [ "${hash}" != "${miniso_hash}" ]; then
    fatal "Streamed hash doesn't match copied hash: ${hash} vs. ${miniso_hash}"
fi

# Check --output-rootfs
rm out
coreos-installer iso extract minimal-iso "${iso}" out --output-rootfs rootfs
hash=$(digest out)
if [ "${hash}" != "${miniso_hash}" ]; then
    fatal "Output hash with rootfs doesn't match copied hash: ${hash} vs. ${miniso_hash}"
fi
hash=$(digest rootfs)
if [ "${hash}" != "${rootfs_hash}" ]; then
    fatal "rootfs hash doesn't match extracted hash: ${hash} vs. ${rootfs_hash}"
fi
rm rootfs
coreos-installer iso extract minimal-iso "${iso}" --output-rootfs rootfs > out
hash=$(digest out)
if [ "${hash}" != "${miniso_hash}" ]; then
    fatal "Streamed hash with rootfs doesn't match copied hash: ${hash} vs. ${miniso_hash}"
fi
hash=$(digest rootfs)
if [ "${hash}" != "${rootfs_hash}" ]; then
    fatal "Streamed rootfs hash doesn't match extracted hash: ${hash} vs. ${rootfs_hash}"
fi

# Check --rootfs-url
rm out
coreos-installer iso extract minimal-iso --rootfs-url https://example.com/rootfs "${iso}" out
modified_hash=$(digest out)
coreos-installer iso kargs show out | grepq "coreos.live.rootfs_url=https://example.com/rootfs"
coreos-installer iso extract minimal-iso --rootfs-url https://example.com/rootfs "${iso}" > out
hash=$(digest out)
if [ "${hash}" != "${modified_hash}" ]; then
    fatal "Streamed hash with rootfs URL doesn't match copied hash: ${hash} vs. ${modified_hash}"
fi

# Output already exists
rm out
(coreos-installer iso extract minimal-iso "${iso}" out --output-rootfs rootfs 2>&1 ||:) | grepq "File exists"
touch out
rm rootfs
(coreos-installer iso extract minimal-iso "${iso}" out --output-rootfs rootfs 2>&1 ||:) | grepq "File exists"

# Corrupt image, ensure extraction fails
sed 's/coreos.liveiso/codeos.liveiso/' "${iso}" > corrupt
rm -f out
(coreos-installer iso extract minimal-iso corrupt out 2>&1 ||:) | grepq "wrong final digest"
(coreos-installer iso extract minimal-iso corrupt 2>&1 > out ||:) | grepq "wrong final digest"

# Customize image, ensure extraction fails
coreos-installer iso kargs modify -a foo=bar "${iso}"
rm out
(coreos-installer iso extract minimal-iso "${iso}" out 2>&1 ||:) | grepq "Cannot operate on ISO with embedded customizations"
coreos-installer iso reset "${iso}"
coreos-installer iso extract minimal-iso "${iso}" out
echo '{"ignition": {"version": "3.3.0"}}' | coreos-installer iso ignition embed "${iso}"
rm out
(coreos-installer iso extract minimal-iso "${iso}" out 2>&1 ||:) | grepq "Cannot operate on ISO with embedded customizations"
coreos-installer iso reset "${iso}"
coreos-installer iso extract minimal-iso "${iso}" out

# Done
echo "Success."
