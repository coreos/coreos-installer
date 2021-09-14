#!/bin/bash
set -xeuo pipefail

fatal() {
    echo "$@" >&2
    exit 1
}

digest() {
    # Ignore filename
    sha256sum "${1:--}" | awk '{print $1}'
}

iso=$1; shift
iso=$(realpath "${iso}")

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

cp --reflink=auto "${iso}" "test.iso"
iso=test.iso
out_iso="${iso}.out"
orig_hash=$(digest "${iso}")

# Sanity-check the ISO doesn't somehow already have the karg we're testing with.
if coreos-installer iso kargs show "${iso}" | grep -q foobar; then
    fatal "Unexpected foobar karg in iso kargs"
fi

# Stream modification to stdout.
stdout_hash=$(coreos-installer iso kargs modify -a foobar=oldval -a dodo -o - "${iso}" | tee "${out_iso}" | digest)
coreos-installer iso kargs show "${out_iso}" | grep -q 'foobar=oldval dodo'
coreos-installer iso kargs modify -d foobar=oldval -d dodo -o - "${out_iso}" > "${iso}"
if coreos-installer iso kargs show "${iso}" | grep -q 'foobar'; then
    fatal "Unexpected foobar karg in iso kargs"
fi
hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Test all the modification operations.
coreos-installer iso kargs modify -a foobar=oldval -a dodo "${iso}"
coreos-installer iso kargs show "${iso}" | grep -q 'foobar=oldval dodo'
hash=$(digest "${iso}")
if [ "${stdout_hash}" != "${hash}" ]; then
    fatal "Streamed hash doesn't match modified hash: ${stdout_hash} vs ${hash}"
fi
rm "${out_iso}"
coreos-installer iso kargs modify -r foobar=oldval=newval "${iso}" -o "${out_iso}"
coreos-installer iso kargs show "${out_iso}" | grep -q 'foobar=newval dodo'
rm "${iso}"
coreos-installer iso kargs modify -d foobar=newval -d dodo "${out_iso}" -o "${iso}"
if coreos-installer iso kargs show "${iso}" | grep -q 'foobar'; then
    fatal "Unexpected foobar karg in iso kargs"
fi

hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Test the largest karg; we get the full area length from --header and subtract
# the default kargs size to get the size of the overflow embed area.
embed_size=$(coreos-installer iso kargs show --header "${iso}" | jq .length)
embed_default_kargs_size=$(coreos-installer iso kargs show --default "${iso}" | wc -c)
embed_usable_size=$((${embed_size} - ${embed_default_kargs_size} - 1))

long_karg=$(printf '%*s' $((embed_usable_size)) | tr ' ' "k")
coreos-installer iso kargs modify -a "${long_karg}" "${iso}"
coreos-installer iso kargs show "${iso}" | grep -q " ${long_karg}\$"
coreos-installer iso kargs reset "${iso}"
long_karg=$(printf '%*s' $((embed_usable_size + 1)) | tr ' ' "k")
if coreos-installer iso kargs modify -a "${long_karg}" "${iso}" 2>err.txt; then
    fatal "Was able to put karg longer than area?"
fi
grep -q 'kargs too large for area' err.txt

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
offset=$(coreos-installer iso kargs show --header "${iso}" | jq -r .kargs[0].offset)
length=$(coreos-installer iso kargs show --header "${iso}" | jq -r .kargs[0].length)
expected_args="$(coreos-installer iso kargs show --default "${iso}") foobar=val"
expected_args_len="$(echo -n "${expected_args}" | wc -c)"
filler="$(printf '%*s' $((${length} - ${expected_args_len} - 1)) | tr ' ' '#')"
if ! echo -en "${expected_args}\n${filler}" | cmp -s <(dd if=${iso} skip=${offset} count=${length} bs=1) -; then
    fatal "Failed to manually round-trip kargs"
fi

# Finally, clobber the header magic and make sure we fail.
dd if=/dev/zero of="${iso}" seek=32672 count=8 bs=1 conv=notrunc status=none
(coreos-installer iso kargs modify -a foobar "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs modify -a foobar "${iso}" -o "${out_iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs modify -a foobar "${iso}" -o - 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs show "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs show --default "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs reset "${iso}" -o - 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs reset "${iso}" -o "${out_iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs reset "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"

# Done
echo "Success."
