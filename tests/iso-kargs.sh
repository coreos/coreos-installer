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

if [ "${iso%.xz}" != "${iso}" ]; then
    xz -dc "${iso}" > test.iso
else
    cp --reflink=auto "${iso}" "test.iso"
fi
iso=test.iso
out_iso="${iso}.out"

# Sanity-check the ISO doesn't somehow already have the karg we're testing with.
if coreos-installer iso kargs show "${iso}" | grep -q foobar; then
    fatal "Unexpected foobar karg in iso kargs"
fi

run_tests() {
    local orig_hash=$(digest "${iso}")

    # Stream modification to stdout.
    local stdout_hash=$(coreos-installer iso kargs modify -a foobar=oldval -a dodo -o - "${iso}" | tee "${out_iso}" | digest)
    coreos-installer iso kargs show "${out_iso}" | grep -q 'foobar=oldval dodo'
    coreos-installer iso kargs modify -d foobar=oldval -d dodo -o - "${out_iso}" > "${iso}"
    if coreos-installer iso kargs show "${iso}" | grep -q 'foobar'; then
        fatal "Unexpected foobar karg in iso kargs"
    fi
    local hash=$(digest "${iso}")
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
    local embed_size=$(coreos-installer iso kargs show --header "${iso}" | jq .length)
    local embed_default_kargs_size=$(coreos-installer iso kargs show --default "${iso}" | wc -c)
    local embed_usable_size=$((${embed_size} - ${embed_default_kargs_size} - 1))

    local long_karg=$(printf '%*s' $((embed_usable_size)) | tr ' ' "k")
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
    local offset=$(coreos-installer iso kargs show --header "${iso}" | jq -r .kargs[0].offset)
    local length=$(coreos-installer iso kargs show --header "${iso}" | jq -r .kargs[0].length)
    local expected_args="$(coreos-installer iso kargs show --default "${iso}") foobar=val"
    local expected_args_len="$(echo -n "${expected_args}" | wc -c)"
    local filler="$(printf '%*s' $((${length} - ${expected_args_len} - 1)) | tr ' ' '#')"
    if ! echo -en "${expected_args}\n${filler}" | cmp -s <(dd if=${iso} skip=${offset} count=${length} bs=1 status=none) -; then
        fatal "Failed to manually round-trip kargs"
    fi

    # Clean up
    coreos-installer iso kargs reset "${iso}"
}

echo "============== Default headers =============="
run_tests

if [ "$(dd if=${iso} skip=32672 count=8 bs=1 status=none | tr -dc [:alnum:])" = "coreKarg" ]; then
    coreKarg=1
    echo "============== Only JSON =============="
    dd if=/dev/zero of="${iso}" seek=32672 count=8 bs=1 conv=notrunc status=none
    run_tests

    echo "============== Only System Area =============="
    echo -n "coreKarg" | dd of="${iso}" seek=32672 bs=1 conv=notrunc status=none
    # Rename KARGS.JSO to XARGS.JSO
    echo -n "X" | dd of="${iso}" seek=$(grep --byte-offset --only-matching --text 'KARGS.JSO;1' "${iso}" | cut -f1 -d:) bs=1 conv=notrunc status=none
    run_tests

    dd if=/dev/zero of="${iso}" seek=32672 count=8 bs=1 conv=notrunc status=none
else
    coreKarg=
    # Rename KARGS.JSO to XARGS.JSO
    echo -n "X" | dd of="${iso}" seek=$(grep --byte-offset --only-matching --text 'KARGS.JSO;1' "${iso}" | cut -f1 -d:) bs=1 conv=notrunc status=none
fi

echo "============== No header =============="
# Make sure we fail
(coreos-installer iso kargs modify -a foobar "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs modify -a foobar "${iso}" -o "${out_iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs modify -a foobar "${iso}" -o - 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs show "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs show --default "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs reset "${iso}" -o - 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs reset "${iso}" -o "${out_iso}" 2>&1 ||:) | grep -q "No karg embed areas found"
(coreos-installer iso kargs reset "${iso}" 2>&1 ||:) | grep -q "No karg embed areas found"

# Done
if [ -n "${coreKarg}" ]; then
    echo "Success; tested with legacy header."
else
    echo "Success.  Legacy header unavailable; tested JSON only."
fi
