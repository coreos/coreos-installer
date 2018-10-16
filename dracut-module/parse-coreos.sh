#!/bin/sh

. /lib/dracut-lib.sh


local IMAGE_URL=$(getarg coreos.image_url=)

if [ $? -eq 0 ]
then
	echo "preset image_url to $IMAGE_URL" >> /tmp/debug
	echo $IMAGE_URL >> /tmp/image_url
fi

local DEST_DEV=$(getarg coreos.install_dev=)

if [ $? -eq 0 ]
then
	echo "preset install dev to $DEST_DEV" >> /tmp/debug
	echo $DEST_DEV >> /tmp/selected_dev
fi

local IGNITION_URL=$(getarg coreos.ignition_url=)
if [ $? -eq 0 ]
then
	echo "preset ignition url to $IGNITION_URL" >> /tmp/debug
	echo $IGNITION_URL >> /dev/ignition_url
fi

local SKIP_VALIDATION=$(getargbool 0 coreos.skip_media_check)
if [ $? -eq 1 ]
then
	echo "Asserting skip of media check" >> /tmp/debug
	touch /tmp/skip_media_check
fi

local SKIP_REBOOT=$(getargbool 0 coreos.skip_reboot)
if [ $? -eq 1 ]
then
	echo "Asserting reboot skip" >> /tmp/debug
	touch /tmp/skip_reboot
fi

# Suppress initrd-switch-root.service from starting
rm -f /etc/initrd-release
