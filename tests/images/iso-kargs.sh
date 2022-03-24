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

# Sanity-check the ISO doesn't somehow already have the karg we're testing with.
if coreos-installer iso kargs show "${iso}" | grepq foobar; then
    fatal "Unexpected foobar karg in iso kargs"
fi

orig_hash=$(digest "${iso}")

# Stream modification to stdout.
stdout_hash=$(coreos-installer iso kargs modify -a foobar=oldval -a dodo -o - "${iso}" | tee "${out_iso}" | digest)
coreos-installer iso kargs show "${out_iso}" | grepq 'foobar=oldval dodo'
coreos-installer iso kargs modify -d foobar=oldval -d dodo -o - "${out_iso}" > "${iso}"
if coreos-installer iso kargs show "${iso}" | grepq 'foobar'; then
    fatal "Unexpected foobar karg in iso kargs"
fi
hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Test all the modification operations.
coreos-installer iso kargs modify -a foobar=oldval -a dodo "${iso}"
coreos-installer iso kargs show "${iso}" | grepq 'foobar=oldval dodo'
hash=$(digest "${iso}")
if [ "${stdout_hash}" != "${hash}" ]; then
    fatal "Streamed hash doesn't match modified hash: ${stdout_hash} vs ${hash}"
fi
rm "${out_iso}"
coreos-installer iso kargs modify -r foobar=oldval=newval "${iso}" -o "${out_iso}"
coreos-installer iso kargs show "${out_iso}" | grepq 'foobar=newval dodo'
rm "${iso}"
coreos-installer iso kargs modify -d foobar=newval -d dodo "${out_iso}" -o "${iso}"
if coreos-installer iso kargs show "${iso}" | grepq 'foobar'; then
    fatal "Unexpected foobar karg in iso kargs"
fi

hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Test the largest karg; we get the full area length from --header and subtract
# the default kargs size to get the size of the overflow embed area.
embed_size=$(coreos-installer dev show iso --kargs "${iso}" | jq .length)
embed_default_kargs_size=$(coreos-installer iso kargs show --default "${iso}" | wc -c)
embed_usable_size=$((${embed_size} - ${embed_default_kargs_size} - 1))

long_karg=$(printf '%*s' $((embed_usable_size)) | tr ' ' "k")
coreos-installer iso kargs modify -a "${long_karg}" "${iso}"
coreos-installer iso kargs show "${iso}" | grepq " ${long_karg}\$"
coreos-installer iso kargs reset "${iso}"
long_karg=$(printf '%*s' $((embed_usable_size + 1)) | tr ' ' "k")
if coreos-installer iso kargs modify -a "${long_karg}" "${iso}" 2>err.txt; then
    fatal "Was able to put karg longer than area?"
fi
grepq 'kargs too large for area' err.txt

# Test `reset`.
coreos-installer iso kargs modify -a foobar "${iso}"
rm "${out_iso}"
coreos-installer iso kargs reset "${iso}" -o "${out_iso}"
hash=$(digest "${out_iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi
coreos-installer iso kargs reset "${iso}" -o - > "${out_iso}"
hash=$(digest "${out_iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi
coreos-installer iso kargs reset "${iso}"
hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Check modification against expected ground truth.
coreos-installer iso kargs modify -a foobar=val "${iso}"
offset=$(coreos-installer dev show iso --kargs "${iso}" | jq -r .kargs[0].offset)
length=$(coreos-installer dev show iso --kargs "${iso}" | jq -r .kargs[0].length)
expected_args="$(coreos-installer iso kargs show --default "${iso}") foobar=val"
expected_args_len="$(echo -n "${expected_args}" | wc -c)"
filler="$(printf '%*s' $((${length} - ${expected_args_len} - 1)) | tr ' ' '#')"
if ! echo -en "${expected_args}\n${filler}" | cmp -s <(dd if=${iso} skip=${offset} count=${length} bs=1 status=none) -; then
    fatal "Failed to manually round-trip kargs"
fi

# Done
echo "Success."
