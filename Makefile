all:
	@echo "(Nothing to do)"

install:
	install -D -m 0755 coreos-installer $(DESTDIR)/usr/libexec/coreos-installer.legacy
	for x in dracut/*; do install -D -t $(DESTDIR)/usr/lib/dracut/modules.d/$$(basename $$x) $$x/*; done
