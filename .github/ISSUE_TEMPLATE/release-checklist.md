# Release process

This project uses [cargo-release][cargo-release] in order to prepare new releases, tag and sign the relevant git commit, and publish the resulting artifacts to [crates.io][crates-io].
The release process follows the usual PR-and-review flow, allowing an external reviewer to have a final check before publishing.

In order to ease downstream packaging of Rust binaries, an archive of vendored dependencies is also provided (only relevant for offline builds).

## Requirements

This guide requires:

 * a web browser (and network connectivity)
 * `git`
 * GPG setup and personal key for signing
 * `cargo` (suggested: latest stable toolchain from [rustup][rustup])
 * `cargo-release` (suggested: `cargo install -f cargo-release`)
 * A verified account on crates.io
 * An account on quay.io
 * Write access to this GitHub project
 * Upload access to this project on GitHub, crates.io, and quay.io

## Release checklist

These steps show how to release version `x.y.z` on the `origin` remote (this can be checked via `git remote -av`).
Push access to the upstream repository is required in order to publish the new tag and the PR branch.

- make sure the project is clean and prepare the environment:
  - [ ] `cargo test`
  - [ ] `cargo clean`
  - [ ] `git clean -fd`
  - [ ] `RELEASE_VER=x.y.z`
  - [ ] `UPSTREAM_REMOTE=origin`

:warning:: `UPSTREAM_REMOTE` should reference the locally configured remote that points to the upstream git repository.

- create release commits on a dedicated branch and tag it:
  - [ ] `git checkout -b release-${RELEASE_VER}`
  - [ ] `cargo release` (and confirm the version when prompted)

- open a PR for this release:
  - [ ] `git push ${UPSTREAM_REMOTE} release-${RELEASE_VER}`
  - [ ] open a web browser and create a PR for the branch above
  - [ ] make sure the resulting PR contains exactly two commits
  - [ ] in the PR body, write a short changelog with relevant changes since last release

- [ ] get the PR reviewed, approved and merged

- publish the artifacts (tag and crate):
  - [ ] `git checkout v${RELEASE_VER}`
  - [ ] verify that `grep "^version = \"${RELEASE_VER}\"$" Cargo.toml` produces output
  - [ ] `git push ${UPSTREAM_REMOTE} v${RELEASE_VER}`
  - [ ] `cargo publish`

- assemble vendor archive:
  - [ ] `cargo vendor`
  - [ ] `tar -czf target/coreos-installer-${RELEASE_VER}-vendor.tar.gz vendor`

- publish this release on GitHub:
  - [ ] open a web browser and [create a GitHub Release](https://github.com/coreos/coreos-installer/releases/new) for the tag above
  - [ ] write a short changelog (i.e. re-use the PR content)
  - [ ] upload `target/coreos-installer-${RELEASE_VER}-vendor.tar.gz`
  - [ ] record digests of local artifacts:
    - `sha256sum target/package/coreos-installer-${RELEASE_VER}.crate`
    - `sha256sum target/coreos-installer-${RELEASE_VER}-vendor.tar.gz`
  - [ ] publish release

- update the `release` tag on Quay:
  - [ ] visit the [Quay tags page](https://quay.io/repository/coreos/coreos-installer?tab=tags) and wait for a versioned tag to appear
  - [ ] click the gear next to the tag, select "Add New Tag", enter `release`, and confirm

- clean up:
  - [ ] `cargo clean`
  - [ ] `rm -rf vendor`
  - [ ] `git checkout master`
  - [ ] `git pull ${UPSTREAM_REMOTE} master`
  - [ ] `git push ${UPSTREAM_REMOTE} :release-${RELEASE_VER}`
  - [ ] `git branch -d release-${RELEASE_VER}`

[cargo-release]: https://github.com/sunng87/cargo-release
[rustup]: https://rustup.rs/
[crates-io]: https://crates.io/
