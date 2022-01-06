#!/bin/bash

dir=$(mktemp -d)
trap "rm -r $dir" EXIT
pushd $dir

mkdir d
echo contents > f
echo ../f | cpio -o -H newc -D d | xz -9c > traversal-relative.img

echo /etc/adjtime | cpio -o -H newc | xz -9c > traversal-absolute.img

popd
mv $dir/*.img .
