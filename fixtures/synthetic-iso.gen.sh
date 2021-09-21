#!/bin/bash
# Regenerate synthetic.iso.xz.

set -xeuo pipefail

rootfs=$(mktemp -d)
trap 'rm -rf "${rootfs}"' EXIT
pushd "${rootfs}" 2>/dev/null

# large dir crossing sectors; directory records can't be smaller than 34 bytes,
# so 150 dirs * 34 = 5100 is the lower bound for the dir, which has to cross
# at least 2 sectors
mkdir largedir
(set +x; for i in {1..150}; do echo 'foo' > "largedir/$i.dat"; done)

# directory with file and dir
mkdir content
echo 'foo' > content/file.txt
mkdir content/dir
echo 'bar' > content/dir/subfile.txt

# deeply nested dirs
mkdir -p really/very/deeply/nested
echo 'foo' > really/very/deeply/nested/file.txt

# naming
mkdir names
touch names/abc
touch names/abc.d
touch names/abc.de
touch names/abc.def
touch names/abcDEFgh
touch names/abcDEFgh.i
touch names/abcDEFgh.ij
touch names/abcDEFgh.ijk
touch names/abcDEFghijkl
touch names/abcDEFghijkl.m
touch names/abcDEFghijkl.mn
touch names/abcDEFghijkl.mno
# this is against ISO-9660, but as genisoimage(1) says:
# "it happens to work on many systems", so let's be resilient
touch 'names/!"%&'\''()*.+,-'
touch 'names/:<=>?'

popd 2>/dev/null
isoname=synthetic.iso
rm -f "${isoname}"{,.xz}
genisoimage \
    -relaxed-filenames \
    -sysid "system-ID-string" \
    -V "volume-ID-string" \
    -o "${isoname}" "${rootfs}"
xz -9 "${isoname}"

# Done
echo "Success."
