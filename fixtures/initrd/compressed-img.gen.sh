#!/bin/bash

dir=$(mktemp -d)
trap "rm -r $dir" EXIT
pushd $dir

make() {
    mkdir "$1"
    echo HELLO > "$1/hello"
    echo WORLD > "$1/world"
    find "$1" | cpio -o -H newc -O "$1.cpio"
}

make uncompressed-1
make uncompressed-2

make gzip
gzip -9 gzip.cpio

make xz
xz -9 xz.cpio

cat uncompressed-1.cpio gzip.cpio.gz xz.cpio.xz uncompressed-2.cpio > compressed.img
xz -9 compressed.img
popd
mv $dir/compressed.img.xz .
