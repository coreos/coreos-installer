RELEASE ?= 0
RDCORE ?= 1

ifeq ($(RELEASE),1)
	PROFILE ?= release
	CARGO_ARGS = --release
	STRIP_RDCORE ?= 0
else
	PROFILE ?= debug
	CARGO_ARGS = --features docgen
	# In debug mode (most often used by devs), we default to stripping
	# `rdcore` because it's otherwise huge and in kola's default 1G VMs can
	# cause ENOSPC. In release mode (most often used by Koji/Brew), we don't do
	# this because the debuginfo gets split out anyway. In either profile,
	# we allow overriding the default.
	STRIP_RDCORE ?= 1
endif
ifeq ($(RDCORE),1)
	CARGO_ARGS := $(CARGO_ARGS) --features rdcore
endif

.PHONY: all
all:
	cargo build ${CARGO_ARGS}
ifneq ($(RDCORE),1)
	rm -f target/$(PROFILE)/rdcore
endif

.PHONY: docs
docs: all data/example-config.yaml
	PROFILE=$(PROFILE) docs/_cmd.sh
	PROFILE=$(PROFILE) docs/_config-file.sh
	target/${PROFILE}/coreos-installer pack man -C man

data/example-config.yaml: target/$(PROFILE)/coreos-installer Makefile
	echo -e "# Sample installer config file\n# Automatically generated; do not edit\n" > $@
	$< pack example-config >> $@

.PHONY: clean
clean:
	cargo clean

.PHONY: install
install: install-bin install-data install-man install-scripts install-systemd install-dracut

.PHONY: install-bin
install-bin:
	install -D -t ${DESTDIR}/usr/bin target/${PROFILE}/coreos-installer

.PHONY: install-data
install-data:
	install -D -m 644 -t ${DESTDIR}/usr/share/coreos-installer data/example-config.yaml

.PHONY: install-man
install-man:
	install -d ${DESTDIR}/usr/share/man/man8
	$(foreach src,$(wildcard man/*.8),gzip -9c $(src) > ${DESTDIR}/usr/share/man/man8/$(notdir $(src)).gz && ) :

.PHONY: install-scripts
install-scripts:
	install -D -t $(DESTDIR)/usr/libexec scripts/coreos-installer-disable-device-auto-activation scripts/coreos-installer-service

.PHONY: install-systemd
install-systemd:
	install -D -m 644 -t $(DESTDIR)/usr/lib/systemd/system systemd/*.{service,target}
	install -D -t $(DESTDIR)/usr/lib/systemd/system-generators systemd/coreos-installer-generator

.PHONY: install-dracut
install-dracut:
	if test -f target/${PROFILE}/rdcore; then \
		for x in dracut/*; do \
			bn=$$(basename $$x); \
			install -D -t $(DESTDIR)/usr/lib/dracut/modules.d/$${bn} $$x/*; \
		done; \
		if [ $(STRIP_RDCORE) -eq 1 ]; then \
			cp target/${PROFILE}/rdcore target/${PROFILE}/rdcore.stripped; \
			strip -g target/${PROFILE}/rdcore.stripped; \
			install -D target/${PROFILE}/rdcore.stripped ${DESTDIR}/usr/lib/dracut/modules.d/50rdcore/rdcore; \
			rm target/${PROFILE}/rdcore.stripped; \
		else \
			install -D -t ${DESTDIR}/usr/lib/dracut/modules.d/50rdcore target/${PROFILE}/rdcore; \
		fi; \
	fi
