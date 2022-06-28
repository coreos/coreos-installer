FROM registry.fedoraproject.org/fedora:36 AS builder
RUN dnf install -y cargo git-core openssl-devel xz-devel
WORKDIR /build
COPY Cargo.* ./
COPY src src/
# Debug symbols are nice but they're not 100+ MB of nice
RUN sed -i 's/^debug = true$/debug = false/' Cargo.toml
# aarch64 release builds running in emulation take too long and time out the
# GitHub Action.  Disable optimization.
RUN if [ $(uname -p) != x86_64 ]; then sed -i "s/^debug = false$/debug = false\nopt-level = 0/" Cargo.toml; fi
# Avoid OOM on emulated arm64
# https://github.com/rust-lang/cargo/issues/10583
RUN mkdir -p .cargo && echo -e '[net]\ngit-fetch-with-cli = true' > .cargo/config.toml
RUN cargo build --release

FROM registry.fedoraproject.org/fedora:36
RUN dnf install -y /usr/bin/gpg /usr/sbin/kpartx /usr/bin/lsblk \
    /usr/sbin/udevadm && \
    dnf clean all
COPY --from=builder /build/target/release/coreos-installer /usr/sbin
ENTRYPOINT ["/usr/sbin/coreos-installer"]
