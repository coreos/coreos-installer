#!/bin/sh

. /lib/dracut-lib.sh


local IMAGE_URL=$(getarg coreos.inst.image_url=)
if [ ! -z "$IMAGE_URL" ]
then
    echo "preset image_url to $IMAGE_URL" >> /tmp/debug
    echo $IMAGE_URL >> /tmp/image_url
fi

local DEST_DEV=$(getarg coreos.inst.install_dev=)
if [ ! -z "$DEST_DEV" ]
then
    echo "preset install dev to $DEST_DEV" >> /tmp/debug
    echo $DEST_DEV >> /tmp/selected_dev
fi

local IGNITION_URL=$(getarg coreos.inst.ignition_url=)
if [ ! -z "$IGNITION_URL" ]
then
    echo "preset ignition url to $IGNITION_URL" >> /tmp/debug
    echo $IGNITION_URL >> /tmp/ignition_url
fi


# Kernel networking args
# Currently only persisting `ipv6.disable`, but additional options may be added
# in the future
# https://github.com/torvalds/linux/blob/master/Documentation/networking/ipv6.txt
declare -a KERNEL_NET_ARGS=("ipv6.disable=")
# Dracut networking args
# Parse all args (other than rd.neednet) and persist those into /tmp/networking_opts
# List from https://www.mankier.com/7/dracut.cmdline#Description-Network
local NETWORKING_ARGS="rd.neednet=1"
declare -a DRACUT_NET_ARGS=("ip=" "ifname=" "rd.route=" "bootdev=" "BOOTIF=" "rd.bootif=" "nameserver=" "rd.peerdns=" "biosdevname=" "vlan=" "bond=" "team=" "bridge=")
for NET_ARG in "${KERNEL_NET_ARGS[@]}" "${DRACUT_NET_ARGS[@]}"
do
    NET_OPT=$(getarg $NET_ARG)
    if [ ! -z "$NET_OPT" ]
    then
        echo "persist $NET_ARG to $NET_OPT" >> /tmp/debug
        NETWORKING_ARGS+=" ${NET_ARG}${NET_OPT}"
    fi
done
# only write /tmp/networking_opts if additional networking options have been specified
# as the default in ignition-dracut specifies `rd.neednet=1 ip=dhcp` when no options are persisted
if [ "${NETWORKING_ARGS}" != "rd.neednet=1" ]
then
    echo "persisting network options: ${NETWORKING_ARGS}" >> /tmp/debug
    echo "${NETWORKING_ARGS}" >> /tmp/networking_opts
fi

if getargbool 0 coreos.inst.skip_media_check
then
    echo "Asserting skip of media check" >> /tmp/debug
    echo 1 > /tmp/skip_media_check
fi

# persist the coreos.no_persist_ip flag if present
if getargbool 0 coreos.no_persist_ip
then
    echo "persisting coreos.no_persist_ip" >> /tmp/debug
    echo "coreos.no_persist_ip=1" >> /tmp/additional_opts
fi

# This one is not consumed by the CLI but actually by the
# coreos-installer.service systemd unit that is run in the
# initramfs. We don't default to rebooting from the CLI.
if getargbool 0 coreos.inst.skip_reboot
then
    echo "Asserting reboot skip" >> /tmp/debug
    echo 1 > /tmp/skip_reboot
fi

if [ "$(getarg coreos.inst=)" = "yes" ]; then
    # Suppress initrd-switch-root.service from starting
    rm -f /etc/initrd-release
    # Suppress most console messages for the installer to run without interference
    dmesg -n 1
fi
