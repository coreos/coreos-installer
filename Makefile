RELEASE ?= 0
RDCORE ?= 1

ifeq ($(RELEASE),1)
	PROFILE ?= release
	CARGO_ARGS = --release
else
	PROFILE ?= debug
	CARGO_ARGS =
endif
ifeq ($(RDCORE),1)
	CARGO_ARGS := $(CARGO_ARGS) --features rdcore
endif

.PHONY: all
all:
	cargo build ${CARGO_ARGS}

.PHONY: install
install: install-bin install-scripts install-systemd install-dracut

.PHONY: install-bin
install-bin: all
	install -D -t ${DESTDIR}/usr/bin target/${PROFILE}/coreos-installer

.PHONY: install-scripts
install-scripts: all
	install -D -t $(DESTDIR)/usr/libexec scripts/coreos-installer-disable-device-auto-activation scripts/coreos-installer-service

.PHONY: install-systemd
install-systemd: all
	install -D -m 644 -t $(DESTDIR)/usr/lib/systemd/system systemd/*.{service,target}
	install -D -t $(DESTDIR)/usr/lib/systemd/system-generators systemd/coreos-installer-generator

.PHONY: install-dracut
install-dracut: all
	if [ "${RDCORE}" = "1" ]; then \
		for x in dracut/*; do \
			bn=$$(basename $$x); \
			install -D -t $(DESTDIR)/usr/lib/dracut/modules.d/$${bn} $$x/*; \
		done; \
		install -D -t ${DESTDIR}/usr/lib/dracut/modules.d/50rdcore target/${PROFILE}/rdcore; \
	fi
