KVER ?= $(shell uname -r)
ISOLINUX ?= /usr/share/syslinux/
help:
	@echo "Targets:"
	@echo "x86_64 - build an eltorito boot iso"
	@echo "help - This message"
	@echo "Variables:"
	@echo "KVER - kernel version to build against"

clean: clean-x86_64

clean-x86_64:
	rm -f ./x86_64/isolinux/initrd.img
	rm -f ./x86_64/isolinux/*.c32
	rm -f ./x86_64/isolinux/*.bin
	rm -f ./x86_64/isolinux/*.png
	rm -f ./x86_64/isolinux/vmlinuz
	rm -f ./x86_64/coreos.iso

x86_64:
	dracut -c ./dracut.conf -f ./x86_64/isolinux/initrd.img ${KVER}
	install -m755 ${ISOLINUX}/isolinux.bin ./x86_64/isolinux/isolinux.bin
	install -m755 ${ISOLINUX}/ldlinux.c32 ./x86_64/isolinux/ldlinux.c32
	install -m755 ${ISOLINUX}/libcom32.c32 ./x86_64/isolinux/libcom32.c32
	install -m755 ${ISOLINUX}/libutil.c32 ./x86_64/isolinux/libutil.c32
	install -m755 ${ISOLINUX}/vesamenu.c32 ./x86_64/isolinux/vesamenu.c32
	convert ./CoreOS.svg ./x86_64/isolinux/splash.png
	install /boot/vmlinuz-${KVER} ./x86_64/isolinux/vmlinuz
	mkisofs -o ./x86_64/coreos.iso -b isolinux.bin -c boot.cat -no-emul-boot -boot-load-size 4 -boot-info-table -R -J -v x86_64/isolinux/

.PHONY: x86_64	
