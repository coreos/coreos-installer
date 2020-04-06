FROM registry.fedoraproject.org/fedora:31 AS builder
RUN dnf install -y cargo openssl-devel
WORKDIR /build
COPY Cargo.* ./
COPY src src/
RUN cargo build --release

FROM registry.fedoraproject.org/fedora:31
RUN dnf install -y /usr/bin/gpg /usr/bin/lsblk /usr/sbin/udevadm /usr/sbin/kpartx && \
    dnf clean all
COPY --from=builder /build/target/release/coreos-installer /usr/sbin
ENTRYPOINT ["/usr/sbin/coreos-installer"]
