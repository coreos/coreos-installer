#!/bin/bash

dir=$(mktemp -d)
trap "rm -r $dir" EXIT
pushd $dir

make() {
    mkdir -p data
    echo "$1" > data/file
    find data | cpio -o -H newc -O "$1.cpio"
}

make first
make second
make third

cat first.cpio second.cpio third.cpio | xz -9c > redundant.img.xz
popd
mv $dir/redundant.img.xz .
