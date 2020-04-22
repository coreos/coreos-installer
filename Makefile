all:
	@echo "(Nothing to do)"

install:
	install -D -t $(DESTDIR)/usr/libexec -m 0755 coreos-installer
	for x in dracut/*; do install -D -t $(DESTDIR)/usr/lib/dracut/modules.d/$$(basename $$x) $$x/*; done
