if systemctl show coreos-installer.service | grep -q "^ActiveState=failed$"; then
    # systemd-udevd seems to be stopped in the emergency shell
    systemctl start systemd-udevd

    # print the coreos-installer help prompt
    /usr/libexec/coreos-installer -h
fi
