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

check() {
    rm -rf extract
    coreos-installer iso network extract -C extract "$1" > /dev/null
    if ! diff -ur src extract; then
        return 1
    fi
}

grepq() {
    # Emulate grep -q without actually using it, to avoid propagating write
    # errors to the writer after a match, which would cause problems with
    # -o pipefail
    grep "$@" > /dev/null
}

iso=$1; shift
iso=$(realpath "${iso}")

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

if [ "${iso%.xz}" != "${iso}" ]; then
    xz -dc "${iso}" > test.iso
else
    cp --reflink=auto "${iso}" "test.iso"
fi
iso=test.iso
out_iso="${iso}.out"
orig_hash=$(digest "${iso}")

mkdir src
cat > src/a.nmconnection <<EOF
[connection]
id=a
EOF
cat > src/b.nmconnection <<EOF
[connection]
id=b
EOF
embed="-k src/b.nmconnection -k src/a.nmconnection"

# Test all the modification operations.
stdout_hash=$(coreos-installer iso network embed ${embed} -o - "${iso}" | tee "${out_iso}" | digest)
check "${out_iso}"
rm "${out_iso}"
coreos-installer iso network embed ${embed} "${iso}" -o "${out_iso}"
check "${out_iso}"
hash=$(digest "${out_iso}")
if [ "${stdout_hash}" != "${hash}" ]; then
    fatal "Streamed hash doesn't match copied hash: ${stdout_hash} vs ${hash}"
fi
coreos-installer iso network embed ${embed} "${iso}"
check "${iso}"
hash=$(digest "${iso}")
if [ "${stdout_hash}" != "${hash}" ]; then
    fatal "Streamed hash doesn't match modified hash: ${stdout_hash} vs ${hash}"
fi

# Test forcing
(coreos-installer iso network embed ${embed} "${iso}" 2>&1 ||:) | grepq "already has embedded network settings"
coreos-installer iso network embed -f ${embed} "${iso}"
rm "${out_iso}"
(coreos-installer iso network embed ${embed} "${iso}" -o "${out_iso}" 2>&1 ||:) | grepq "already has embedded network settings"
coreos-installer iso network embed -f ${embed} "${iso}" -o "${out_iso}"
(coreos-installer iso network embed ${embed} "${iso}" -o - 2>&1 ||:) | grepq "already has embedded network settings"
coreos-installer iso network embed -f ${embed} "${iso}" -o - >/dev/null

# Test `extract` to stdout
coreos-installer iso network extract "${iso}" | grepq "id=a"
coreos-installer iso network extract "${iso}" | grepq "id=b"

# Test `remove`
hash=$(coreos-installer iso network remove "${iso}" -o - | digest)
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi
rm "${out_iso}"
coreos-installer iso network remove "${iso}" -o "${out_iso}"
hash=$(digest "${out_iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi
coreos-installer iso network remove "${iso}"
hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Check that network configs work independently of Ignition configs
echo '{"ignition": {"version": "3.0.0"}' | coreos-installer iso ignition embed "${iso}"
(coreos-installer iso network extract "${iso}" 2>&1 ||:) | grepq "No embedded network settings"
rm "${out_iso}"
coreos-installer iso network embed ${embed} "${iso}" -o "${out_iso}"
check "${out_iso}"
coreos-installer iso ignition show "${out_iso}" | grepq "version"
coreos-installer iso network embed ${embed} "${iso}"
coreos-installer iso ignition show "${iso}" | grepq "version"
rm "${out_iso}"
coreos-installer iso network remove "${iso}" -o "${out_iso}"
coreos-installer iso ignition show "${out_iso}" | grepq "version"
coreos-installer iso network remove "${iso}"
coreos-installer iso ignition show "${iso}" | grepq "version"
(coreos-installer iso network extract "${iso}" 2>&1 ||:) | grepq "No embedded network settings"
coreos-installer iso ignition remove "${iso}"
# verify we haven't written an empty cpio archive
offset=$(coreos-installer dev show iso --ignition "${iso}" | jq -r .offset)
length=$(coreos-installer dev show iso --ignition "${iso}" | jq -r .length)
dd if="${iso}" skip="${offset}" count="${length}" bs=1 status=none | cmp -n "${length}" - /dev/zero
rm "${out_iso}"

# Clobber the **kargs** header magic and make sure we still succeed
dd if=/dev/zero of="${iso}" seek=32672 count=8 bs=1 conv=notrunc status=none
coreos-installer iso network embed ${embed} "${iso}" -o "${out_iso}"
coreos-installer iso network embed ${embed} "${iso}" -o - >/dev/null
coreos-installer iso network embed ${embed} "${iso}"
coreos-installer iso network extract "${iso}" >/dev/null
coreos-installer iso network remove "${iso}" -o - >/dev/null
rm "${out_iso}"
coreos-installer iso network remove "${iso}" -o "${out_iso}"
coreos-installer iso network remove "${iso}"

# Done
echo "Success."
