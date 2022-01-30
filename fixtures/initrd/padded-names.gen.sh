#!/bin/bash
# Use dracut-cpio to generate an archive with padded name fields.

dir=$(mktemp -d)
trap "rm -r $dir" EXIT
pushd $dir

mkdir dir
# dracut-cpio won't pad unless file size is larger than requested alignment
printf 'z%.0s' {1..5000} > dir/hello
printf 'q%.0s' {1..4500} > dir/world
find dir | dracut-cpio --data-align=4096 --mtime=0 --owner=0:0 padded-names.img
xz -9 padded-names.img

popd
mv $dir/padded-names.img.xz .
