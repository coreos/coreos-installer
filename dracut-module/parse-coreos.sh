#!/bin/sh

. /lib/dracut-lib.sh


local IMAGE_URL=$(getarg coreos.image_url=)
if [ ! -z "$IMAGE_URL" ]
then
	echo "preset image_url to $IMAGE_URL" >> /tmp/debug
	echo $IMAGE_URL >> /tmp/image_url
fi

local DEST_DEV=$(getarg coreos.install_dev=)
if [ ! -z "$DEST_DEV" ]
then
	echo "preset install dev to $DEST_DEV" >> /tmp/debug
	echo $DEST_DEV >> /tmp/selected_dev
fi

local IGNITION_URL=$(getarg coreos.ignition_url=)
if [ ! -z "$IGNITION_URL" ]
then
	echo "preset ignition url to $IGNITION_URL" >> /tmp/debug
	echo $IGNITION_URL >> /dev/ignition_url
fi

if getargbool 0 coreos.skip_media_check
then
	echo "Asserting skip of media check" >> /tmp/debug
	echo 1 > /tmp/skip_media_check
fi

if getargbool 0 coreos.skip_reboot 
then
	echo "Asserting reboot skip" >> /tmp/debug
	echo 1 > /tmp/skip_reboot
fi

# Suppress initrd-switch-root.service from starting
rm -f /etc/initrd-release

# Suppress most console messages for the installer to run without interference
dmesg -n 1
