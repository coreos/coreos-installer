#!/bin/bash
set -xeuo pipefail
PS4='${LINENO}: '

timeout=10m

rootdir="$(realpath $(dirname $0)/../..)"
fixtures="${rootdir}/fixtures/customize"

tmpd=$(mktemp -d)
trap 'rm -rf "${tmpd}"' EXIT
cd "${tmpd}"

artifactdir=$1; shift
ln -s ${artifactdir}/*-live-kernel-x86_64 src-kernel
ln -s ${artifactdir}/*-live-initramfs.x86_64.img src-initrd
ln -s ${artifactdir}/*-live-rootfs.x86_64.img src-rootfs
ln -s ${artifactdir}/*-live.x86_64.iso src-iso

digest() {
    # Ignore filename
    sha256sum "${1:--}" | awk '{print $1}'
}

grepq() {
    # Emulate grep -q without actually using it, to avoid propagating write
    # errors to the writer after a match, which would cause problems with
    # -o pipefail
    grep "$@" > /dev/null
}

# kargs we need on every boot
kargs_common=(
    # make sure we get log output
    console=ttyS0,115200
    # make sure it's readable
    systemd.log_color=0
    # force NM in initrd
    rd.neednet=1
)

# options that don't initiate an install, even if they pertain to one
opts_common=(
    # @applied-live-ign@
    # @did-not-install@ if that's true
    --live-ignition "${fixtures}/live.ign"
    # @applied-live-2-ign@
    --live-ignition "${fixtures}/live-2.ign"
    # @preinst-1@
    --pre-install "${fixtures}/pre-install-1"
    # @preinst-2@
    --pre-install "${fixtures}/pre-install-2"
    # @postinst-1@
    --post-install "${fixtures}/post-install-1"
    # @postinst-2@
    --post-install "${fixtures}/post-install-2"
    # 'Adding "coreos-installer test certificate" to list of CAs'
    --ignition-ca "${fixtures}/cert.pem"
    # Condition of @applied-live-ign@ and @applied-dest-ign@
    --network-keyfile "${fixtures}/installer-test.nmconnection"
)

# options that do initiate an install
opts_install=(
    # @applied-dest-ign@
    --dest-ignition "${fixtures}/dest.ign"
    # @applied-dest-2-ign@
    --dest-ignition "${fixtures}/dest-2.ign"
    # Condition of @applied-dest-ign@
    --installer-config "${fixtures}/install-1.conf"
    # Condition of @applied-dest-ign@
    --installer-config "${fixtures}/install-2.conf"
    # Tested implicitly
    --dest-device "/dev/vda"
    # Condition of @applied-dest-ign@
    --dest-karg-append dest-karg
    # Condition of @applied-dest-ign@
    --dest-karg-delete ignition.platform.id=metal
    # Condition of @applied-dest-ign@
    --dest-karg-append ignition.platform.id=qemu
)
for arg in ${kargs_common[@]}; do
    opts_install+=(--dest-karg-append "${arg}")
done

opts_iso=()
for arg in ${kargs_common[@]}; do
    opts_iso+=(--live-karg-append "${arg}")
done

iso_customize() {
    rm -f iso
    coreos-installer iso customize src-iso -o iso "${opts_iso[@]}" "$@"
}

pxe_customize() {
    rm -f initrd
    coreos-installer pxe customize src-initrd -o initrd "$@"
}

qemu_common() {
    timeout --foreground "${timeout}" \
        qemu-system-x86_64 \
        -m 4096 \
        -accel kvm \
        -object rng-random,filename=/dev/urandom,id=rng0 \
        -netdev user,id=eth0,hostname="fcos",tftp=.,bootfile=ipxe \
        -device virtio-net-pci,netdev=eth0 \
        -nographic \
        -no-reboot \
        "$@" < /dev/null | tee log
}

qemu_iso() {
    qemu-img create -f qcow2 disk 8G
    qemu_common \
        -drive file=disk,if=virtio,format=qcow2,cache=unsafe \
        -drive file=iso,if=ide,format=raw,media=cdrom,cache=unsafe
}

qemu_pxe() {
    cat > ipxe <<EOF
#!ipxe
kernel tftp://10.0.2.2/src-kernel ignition.firstboot ignition.platform.id=qemu ${kargs_common[*]}
initrd tftp://10.0.2.2/initrd
initrd tftp://10.0.2.2/src-rootfs
boot
EOF
    qemu-img create -f qcow2 disk 8G
    qemu_common -drive file=disk,if=virtio,format=qcow2,cache=unsafe
    rm ipxe
}

qemu_disk() {
    qemu_common -drive file=disk,if=virtio,format=qcow2,cache=unsafe
}

assert() {
    grep -Fq "$1" log
}

check_live_noinstall() {
    assert @applied-live-ign@
    assert @applied-live-2-ign@
    ! assert @applied-dest-ign@
    ! assert @applied-dest-2-ign@
    assert @did-not-install@
    ! assert @preinst-1@
    ! assert @preinst-2@
    ! assert @postinst-1@
    ! assert @postinst-2@
    assert 'Adding "coreos-installer test certificate" to list of CAs'
}

check_live_install() {
    assert @applied-live-ign@
    assert @applied-live-2-ign@
    ! assert @applied-dest-ign@
    ! assert @applied-dest-2-ign@
    ! assert @did-not-install@
    assert @preinst-1@
    assert @preinst-2@
    assert @postinst-1@
    assert @postinst-2@
    assert 'Adding "coreos-installer test certificate" to list of CAs'
}

check_dest() {
    ! assert @applied-live-ign@
    ! assert @applied-live-2-ign@
    assert @applied-dest-ign@
    assert @applied-dest-2-ign@
    ! assert @preinst-1@
    ! assert @preinst-2@
    ! assert @postinst-1@
    ! assert @postinst-2@
    assert 'Adding "coreos-installer test certificate" to list of CAs'
}

# Check equivalence of ISO outputs
coreos-installer iso customize src-iso -o iso \
    "${opts_common[@]}" "${opts_install[@]}"
expected=$(digest iso)
cp --dereference --reflink=auto src-iso inplace-iso
coreos-installer iso customize inplace-iso \
    "${opts_common[@]}" "${opts_install[@]}"
[ "${expected}" = "$(digest inplace-iso)" ]
rm inplace-iso
found=$(coreos-installer iso customize src-iso -o - \
    "${opts_common[@]}" "${opts_install[@]}" | digest)
[ "${expected}" = "${found}" ]

# Check ISO error conditions
(coreos-installer iso customize src-iso -o iso \
    "${opts_common[@]}" "${opts_install[@]}" 2>&1 ||:) |
    grepq "File exists"
(coreos-installer iso customize iso \
    "${opts_common[@]}" "${opts_install[@]}" 2>&1 ||:) |
    grepq "already customized"
(coreos-installer iso customize iso -o iso2 \
    "${opts_common[@]}" "${opts_install[@]}" 2>&1 ||:) |
    grepq "already customized"
rm iso
xz -dc "${rootdir}/fixtures/iso/embed-areas-2021-09.iso.xz" > old.iso
(coreos-installer iso customize old.iso \
    --network-keyfile "${fixtures}/installer-test.nmconnection" 2>&1 ||:) |
    grepq "does not support customizing network settings"
(coreos-installer iso customize old.iso --dest-device /dev/loop0 2>&1 ||:) |
    grepq "does not support customizing installer configuration"
coreos-installer iso customize old.iso \
    --pre-install "${fixtures}/pre-install-1" \
    --live-karg-append "foo"
xz -dc "${rootdir}/fixtures/iso/embed-areas-2020-09.iso.xz" > old.iso
(coreos-installer iso customize old.iso \
    --live-karg-append "foo" 2>&1 ||:) |
    grepq "does not support customizing live kernel arguments"
coreos-installer iso customize old.iso \
    --pre-install "${fixtures}/pre-install-1"
xz -dc "${rootdir}/fixtures/iso/synthetic.iso.xz" > old.iso
(coreos-installer iso customize old.iso \
    --pre-install "${fixtures}/pre-install-1" 2>&1 ||:) |
    grepq "Unrecognized CoreOS ISO image"
# no-op
coreos-installer iso customize src-iso -o iso

# Check PXE initrd concatenation
pxe_customize "${opts_common[@]}" "${opts_install[@]}"
orig_size=$(stat -Lc %s src-initrd)
# head part
cmp -n "${orig_size}" src-initrd initrd
# tail part
[ $(dd if=initrd skip="$((${orig_size} + 1))" bs=1 count=4 status=none) = 7zXZ ]
rm initrd

# Check equivalence of PXE outputs
coreos-installer pxe customize src-initrd -o initrd \
    "${opts_common[@]}" "${opts_install[@]}"
expected=$(digest initrd)
found=$(coreos-installer pxe customize src-initrd -o - \
    "${opts_common[@]}" "${opts_install[@]}" | digest)
[ "${expected}" = "${found}" ]

# Check PXE error conditions
# don't re-test feature flags here, since the comprehensive tests would fail
# if flags weren't being read correctly
(coreos-installer pxe customize src-initrd -o initrd \
    "${opts_common[@]}" "${opts_install[@]}" 2>&1 ||:) |
    grepq "File exists"
(coreos-installer pxe customize initrd -o initrd2 \
    "${opts_common[@]}" "${opts_install[@]}" 2>&1 ||:) |
    grepq "already customized"
rm initrd
coreos-installer pxe ignition wrap -i /dev/null > empty-initrd
(coreos-installer pxe customize empty-initrd -o initrd 2>&1 ||:) |
    grepq "not a CoreOS live initramfs image"
# no-op
coreos-installer pxe customize src-initrd -o initrd

# Check arg restrictions
(iso_customize \
    --pre-install "${fixtures}/pre-install-1" \
    --pre-install "${fixtures}/pre-install-1" 2>&1 ||:) |
    grepq "already specifies path"
(iso_customize \
    --network-keyfile "${fixtures}/installer-test.nmconnection" \
    --network-keyfile "${fixtures}/installer-test.nmconnection" 2>&1 ||:) |
    grepq "already specifies keyfile"
(iso_customize \
    --live-ignition "${fixtures}/installer-test.nmconnection"  2>&1 ||:) |
    grepq "parsing Ignition config"
(iso_customize \
    --dest-ignition "${fixtures}/installer-test.nmconnection" 2>&1 ||:) |
    grepq "parsing Ignition config"
(iso_customize \
    --installer-config "${fixtures}/installer-test.nmconnection" 2>&1 ||:) |
    grepq "parsing installer config"

# Test live kargs by reading them back out of the ISO
coreos-installer iso kargs show src-iso | grepq ignition.platform.id=metal
iso_customize \
    --live-karg-append foo \
    --live-karg-replace ignition.platform.id=metal=bar
coreos-installer iso kargs show iso | grepq ignition.platform.id=bar
coreos-installer iso kargs show iso | grepq foo
iso_customize \
    --live-karg-delete ignition.platform.id=metal
! coreos-installer iso kargs show iso | grepq ignition.platform.id

# Runtime tests
echo "=== ISO without install ==="
iso_customize "${opts_common[@]}"
qemu_iso
check_live_noinstall

echo "=== ISO with install ==="
iso_customize "${opts_common[@]}" "${opts_install[@]}"
qemu_iso
check_live_install
qemu_disk
check_dest

# User config passed directly to installer without wrapping
echo "=== ISO with one dest config ==="
iso_customize --dest-ignition "${fixtures}/dest-2.ign" --dest-device /dev/vda
qemu_iso
qemu_disk
assert @applied-dest-2-ign@

echo "=== PXE without install ==="
pxe_customize "${opts_common[@]}"
qemu_pxe
check_live_noinstall

echo "=== PXE with install ==="
pxe_customize "${opts_common[@]}" "${opts_install[@]}"
qemu_pxe
check_live_install
qemu_disk
check_dest

# Done
echo "Success."
