#!/usr/bin/env bash

cmd="../target/release/coreos-installer"
version="0.4.0"

# Standard commands
help2man --no-info --section=1 --version-string="${version}" "${cmd}"              --output=coreos-installer.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} install"      --output=coreos-installer-install.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} download"     --output=coreos-installer-download.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} list-stream"  --output=coreos-installer-list-stream.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} iso"          --output=coreos-installer-iso.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} iso embed"    --output=coreos-installer-iso-embed.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} iso show"     --output=coreos-installer-iso-show.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} iso remove"   --output=coreos-installer-iso-remove.1

# Hidden command
help2man --no-info --section=1 --version-string="${version}" "${cmd} osmet"        --output=coreos-installer-osmet.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} osmet pack"   --output=coreos-installer-osmet-pack.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} osmet unpack" --output=coreos-installer-osmet-unpack.1
help2man --no-info --section=1 --version-string="${version}" "${cmd} osmet fiemap" --output=coreos-installer-osmet-fiemap.1

# Fixup section
for f in ./coreos-installer-*.1; do 
	sed -i "s/.TH COREOS-INSTALLER [A-Z ]*/.TH COREOS-INSTALLER /g" "${f}"
done
