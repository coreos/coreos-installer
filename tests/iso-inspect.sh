#!/bin/bash
set -xeuo pipefail

fatal() {
    echo "$@" >&2
    exit 1
}

iso=$1; shift
iso=$(realpath "${iso}")

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

coreos-installer iso inspect "${iso}" | tee inspect.json

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

# now test some custom ISOs

# large dir crossing sectors; directory records can't be smaller than 34 bytes,
# so 150 dirs * 34 = 5100 is the lower bound for the root dir, which has to
# cross at least 2 sectors
mkdir rootfs
(set +x; for i in {1..150}; do echo 'foo' > "rootfs/$i.dat"; done)
genisoimage -o testiso.iso rootfs
coreos-installer iso inspect "testiso.iso" | tee inspect.json
jq -e '.records|length == 150' inspect.json

# deeply nested dirs
rm -rf rootfs testiso.iso
mkdir -p rootfs/really/very/deeply/nested
echo 'foo' > rootfs/really/very/deeply/nested/file.txt

# naming
mkdir rootfs/names
touch rootfs/names/abc
touch rootfs/names/abc.d
touch rootfs/names/abc.de
touch rootfs/names/abc.def
touch rootfs/names/abcDEFgh
touch rootfs/names/abcDEFgh.i
touch rootfs/names/abcDEFgh.ij
touch rootfs/names/abcDEFgh.ijk
touch rootfs/names/abcDEFghijkl
touch rootfs/names/abcDEFghijkl.m
touch rootfs/names/abcDEFghijkl.mn
touch rootfs/names/abcDEFghijkl.mno
# this is against ISO-9660, but as genisoimage(1) says:
# "it happens to work on many systems", so let's be resilient
touch 'rootfs/names/!"%&'\''()*.+,-'
touch 'rootfs/names/:<=>?'

genisoimage -relaxed-filenames -o testiso.iso rootfs
coreos-installer iso inspect "testiso.iso" | tee inspect.json

jq -e '.records|index("REALLY") >= 0' inspect.json
jq -e '.records|index("REALLY/VERY") >= 0' inspect.json
jq -e '.records|index("REALLY/VERY/DEEPLY") >= 0' inspect.json
jq -e '.records|index("REALLY/VERY/DEEPLY/NESTED") >= 0' inspect.json
jq -e '.records|index("REALLY/VERY/DEEPLY/NESTED/FILE.TXT") >= 0' inspect.json
jq -e '.records|index("NAMES") >= 0' inspect.json
jq -e '.records|index("NAMES/ABC") >= 0' inspect.json
jq -e '.records|index("NAMES/ABC.D") >= 0' inspect.json
jq -e '.records|index("NAMES/ABC.DE") >= 0' inspect.json
jq -e '.records|index("NAMES/ABC.DEF") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH.I") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH.IJ") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH.IJK") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDE000") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH.M") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH.MN") >= 0' inspect.json
jq -e '.records|index("NAMES/ABCDEFGH.MNO") >= 0' inspect.json
jq -e '.records|index("NAMES/:<=>?") >= 0' inspect.json
jq -e '.records|index("NAMES/!\"%&'\''()*.+,-") >= 0' inspect.json

# Done
echo "Success."
