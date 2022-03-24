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
orig_hash=$(digest "${iso}")

config='{"ignition": {"version": "3.0.0"}'

# Test all the modification operations.
stdout_hash=$(echo "${config}" | coreos-installer iso ignition embed -o - "${iso}" | tee "${out_iso}" | digest)
coreos-installer iso ignition show "${out_iso}" | cmp - <(echo "${config}")
rm "${out_iso}"
coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -o "${out_iso}"
coreos-installer iso ignition show "${out_iso}" | cmp - <(echo "${config}")
hash=$(digest "${out_iso}")
if [ "${stdout_hash}" != "${hash}" ]; then
    fatal "Streamed hash doesn't match copied hash: ${stdout_hash} vs ${hash}"
fi
coreos-installer iso ignition embed -i <(echo "${config}") "${iso}"
coreos-installer iso ignition show "${iso}" | cmp - <(echo "${config}")
hash=$(digest "${iso}")
if [ "${stdout_hash}" != "${hash}" ]; then
    fatal "Streamed hash doesn't match modified hash: ${stdout_hash} vs ${hash}"
fi

# Check the actual file bits.
offset=$(coreos-installer dev show iso --ignition "${iso}" | jq -r .offset)
length=$(coreos-installer dev show iso --ignition "${iso}" | jq -r .length)
if [ "${config}" != "$(dd if=${iso} skip=${offset} count=${length} bs=1 status=none | xzcat | cpio -i --to-stdout --quiet)" ]; then
    fatal "Failed to manually round-trip Ignition config"
fi

# Test forcing
(coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" 2>&1 ||:) | grepq "already has an embedded Ignition config"
coreos-installer iso ignition embed -f -i <(echo "${config}") "${iso}"
rm "${out_iso}"
(coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -o "${out_iso}" 2>&1 ||:) | grepq "already has an embedded Ignition config"
coreos-installer iso ignition embed -f -i <(echo "${config}") "${iso}" -o "${out_iso}"
(coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -o - 2>&1 ||:) | grepq "already has an embedded Ignition config"
coreos-installer iso ignition embed -f -i <(echo "${config}") "${iso}" -o - >/dev/null

# Test `remove`
hash=$(coreos-installer iso ignition remove "${iso}" -o - | digest)
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi
rm "${out_iso}"
coreos-installer iso ignition remove "${iso}" -o "${out_iso}"
hash=$(digest "${out_iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi
coreos-installer iso ignition remove "${iso}"
hash=$(digest "${iso}")
if [ "${orig_hash}" != "${hash}" ]; then
    fatal "Hash doesn't match original hash: ${hash} vs ${orig_hash}"
fi

# Test an overlarge Ignition config.  Get some random data from /dev/urandom
# to ensure it's sufficiently incompressible.
embed_size=$(coreos-installer dev show iso --ignition "${iso}" | jq .length)
set +x
random=$(dd if=/dev/urandom bs=1 count=${embed_size} status=none | base64 -w0)
set -x
large_config() {
    # too large for sed argument list
    cat <<EOF
{"ignition": {"version": "3.0.0"}, "storage": {"files": [{"path": "/etc/foo", "contents": {"source": "data:,${random}"}}]}}
EOF
}
(large_config | coreos-installer iso ignition embed -o - "${iso}" 2>&1 ||:) | grepq "too large"
rm "${out_iso}"
(large_config | coreos-installer iso ignition embed -o "${out_iso}" "${iso}" 2>&1 ||:) | grepq "too large"
(large_config | coreos-installer iso ignition embed "${iso}" 2>&1 ||:) | grepq "too large"

# Check that Ignition configs work independently of network configs
echo "foo=baz" > one.nmconnection
echo "bar=baz" > two.nmconnection
if coreos-installer iso network embed -k one.nmconnection -k two.nmconnection "${iso}"; then
    (coreos-installer iso ignition show "${iso}" 2>&1 ||:) | grepq "No embedded Ignition config"
    coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -o "${out_iso}"
    coreos-installer iso ignition show "${out_iso}" | cmp - <(echo "${config}")
    coreos-installer iso network extract "${out_iso}" | grepq "foo=baz"
    coreos-installer iso network extract "${out_iso}" | grepq "bar=baz"
    coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -f
    coreos-installer iso network extract "${out_iso}" | grepq "foo=baz"
    rm "${out_iso}"
    coreos-installer iso ignition remove "${iso}" -o "${out_iso}"
    coreos-installer iso network extract "${out_iso}" | grepq "foo=baz"
    coreos-installer iso ignition remove "${iso}"
    coreos-installer iso network extract "${out_iso}" | grepq "foo=baz"
    (coreos-installer iso ignition show "${iso}" 2>&1 ||:) | grepq "No embedded Ignition config"
    coreos-installer iso network remove "${iso}"
    # verify we haven't written an empty cpio archive
    dd if="${iso}" skip="${offset}" count="${length}" bs=1 status=none | cmp -n "${length}" - /dev/zero
    rm "${out_iso}"
else
    echo "Failed to embed network settings; skipping"
fi

# Clobber the **kargs** header magic and make sure we still succeed
dd if=/dev/zero of="${iso}" seek=32672 count=8 bs=1 conv=notrunc status=none
coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -o "${out_iso}"
coreos-installer iso ignition embed -i <(echo "${config}") "${iso}" -o - >/dev/null
coreos-installer iso ignition embed -i <(echo "${config}") "${iso}"
coreos-installer iso ignition show "${iso}" >/dev/null
coreos-installer iso ignition remove "${iso}" -o - >/dev/null
rm "${out_iso}"
coreos-installer iso ignition remove "${iso}" -o "${out_iso}"
coreos-installer iso ignition remove "${iso}"

# Done
echo "Success."
