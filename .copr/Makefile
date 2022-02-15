.PHONY: srpm
srpm:
	dnf install -y git rust-packaging rpm-build rpmdevtools
	curl -LO https://src.fedoraproject.org/rpms/rust-coreos-installer/raw/rawhide/f/rust-coreos-installer.spec
	version=$$(git describe --always --tags | sed -e 's,-,\.,g' -e 's,^v,,'); \
	git archive --format=tar --prefix=coreos-installer-$$version/ HEAD | gzip > coreos-installer-$$version.crate; \
	sed -ie "s,^Version:.*,Version: $$version," rust-coreos-installer.spec
	sed -ie 's/^Patch/# Patch/g' rust-coreos-installer.spec  # we don't want any downstream patches
	sed -ie 's/^Source1/# Source1/g' rust-coreos-installer.spec  # we don't vendor
	spectool -g -s 2 rust-coreos-installer.spec  # fetch coreos-installer-dracut just to satisfy rpmbuild
	rpmbuild -bs --define "_sourcedir ${PWD}" --define "_specdir ${PWD}" --define "_builddir ${PWD}" --define "_srcrpmdir ${PWD}" --define "_rpmdir ${PWD}" --define "_buildrootdir ${PWD}/.build" rust-coreos-installer.spec
	mv *.src.rpm $$outdir
