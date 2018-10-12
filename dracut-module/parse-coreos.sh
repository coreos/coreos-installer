#!/bin/sh

. /lib/dracut-lib.sh


local IMAGE_URL=$(getarg coreos.image_url=)

if [ $? -eq 0 ]
then
	echo $IMAGE_URL >> /tmp/image_url
fi

local DEST_DEV=$(getarg coreos.install_dev=)

if [ $? -eq 0 ]
then
	echo $DEST_DEV >> /tmp/selected_dev
fi

local IGNITION_URL=$(getarg coreos.ignition_url)
if [ $? -eq 0 ]
then
	echo $IGNITION_URL >> /dev/ignition_url
fi

# Suppress initrd-switch-root.service from starting
rm -f /etc/initrd-release
