#!/bin/bash
set -xeuo pipefail
PS4='${LINENO}: '

fixtures="$(realpath $(dirname $0)/../..)/fixtures"

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

xz -dc "${fixtures}/initrd/compressed.img.xz" > compressed.img

files() {
    cat <<EOF
gzip/hello
gzip/world
uncompressed-1/hello
uncompressed-1/world
uncompressed-2/hello
uncompressed-2/world
xz/hello
xz/world
EOF
}

check() {
    pushd "$1" >/dev/null
    shift
    for dir in "$@"; do
        echo HELLO | diff "$dir/hello" -
        echo WORLD | diff "$dir/world" -
    done
    popd >/dev/null
}

grepq() {
    # Emulate grep -q without actually using it, to avoid propagating write
    # errors to the writer after a match, which would cause problems with
    # -o pipefail
    grep "$@" > /dev/null
}

# dev show initrd
coreos-installer dev show initrd compressed.img > out
files | diff - out
coreos-installer dev show initrd - < compressed.img > out
files | diff - out
coreos-installer dev show initrd compressed.img '*hello' > out
files | grep hello | diff - out
coreos-installer dev show initrd compressed.img 'gzip/*' > out
files | grep gzip | diff - out
coreos-installer dev show initrd - 'gzip*' 'xz*' < compressed.img > out
files | grep -E 'gzip|xz' | diff - out

# dev extract initrd
coreos-installer dev extract initrd compressed.img
check . gzip uncompressed-1 uncompressed-2 xz
(coreos-installer dev extract initrd compressed.img 2>&1 ||:) | grepq exists
rm -r gzip uncompressed-[12] xz
coreos-installer dev extract initrd -C d - < compressed.img
check d gzip uncompressed-1 uncompressed-2 xz
rm -r d
coreos-installer dev extract initrd -C d -v compressed.img > out
files | sed s:^:d/: | diff - out
check d gzip uncompressed-1 uncompressed-2 xz
rm -r d
coreos-installer dev extract initrd -C d -v compressed.img 'gzip/*' 'xz/*' > out
files | sed s:^:d/: | grep -E 'gzip|xz' | diff - out
check d gzip xz
[ -e d/uncompressed-1 ] && exit 1
[ -e d/uncompressed-2 ] && exit 1
rm -r d
(coreos-installer dev extract initrd \
    "${fixtures}/initrd/traversal-absolute.img" 2>&1 ||:) | grepq traversal
(coreos-installer dev extract initrd \
    "${fixtures}/initrd/traversal-relative.img" 2>&1 ||:) | grepq traversal

# Done
echo "Success."
