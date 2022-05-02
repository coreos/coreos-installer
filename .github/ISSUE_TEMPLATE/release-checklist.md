# Release process

This project uses [cargo-release][cargo-release] in order to prepare new releases, tag and sign the relevant git commit, and publish the resulting artifacts to [crates.io][crates-io].
The release process follows the usual PR-and-review flow, allowing an external reviewer to have a final check before publishing.

In order to ease downstream packaging of Rust binaries, an archive of vendored dependencies is also provided (only relevant for offline builds).

## Requirements

This guide requires:

 * A web browser (and network connectivity)
 * `git`
 * [GPG setup][GPG setup] and personal key for signing
 * `cargo` (suggested: latest stable toolchain from [rustup][rustup])
 * `cargo-release` (suggested: `cargo install -f cargo-release`)
 * A verified account on crates.io
 * An account on quay.io
 * Write access to this GitHub project
 * Upload access to this project on GitHub and and quay.io
 * Membership in the [Fedora CoreOS Crates Owners group](https://github.com/orgs/coreos/teams/fedora-coreos-crates-owners/members), which will give you upload access to crates.io

## Release checklist

These steps show how to release version `x.y.z` on the `origin` remote (this can be checked via `git remote -av`).
Push access to the upstream repository is required in order to publish the new tag and the PR branch.

:warning:: if `origin` is not the name of the locally configured remote that points to the upstream git repository (i.e. `git@github.com:coreos/coreos-installer.git`), be sure to assign the correct remote name to the `UPSTREAM_REMOTE` variable.

- make sure the project is clean and prepare the environment:
  - [ ] Make sure `cargo-release` is up to date: `cargo install cargo-release`
  - [ ] `cargo test --all-features`
  - [ ] `cargo clean`
  - [ ] `git clean -fd`
  - [ ] `RELEASE_VER=x.y.z`
  - [ ] `UPSTREAM_REMOTE=origin`

- create release commits on a dedicated branch and tag it (the commits and tag will be signed with the GPG signing key you configured):
  - [ ] `git checkout -b release-${RELEASE_VER}`
  - [ ] `cargo release --execute ${RELEASE_VER}` (and confirm the version when prompted)

- open and merge a PR for this release:
  - [ ] `git push ${UPSTREAM_REMOTE} release-${RELEASE_VER}`
  - [ ] open a web browser and create a PR for the branch above
  - [ ] make sure the resulting PR contains exactly one commit
  - [ ] in the PR body, write a short changelog with relevant changes since last release
  - [ ] get the PR reviewed, approved and merged

- publish the artifacts (tag and crate):
  - [ ] `git checkout v${RELEASE_VER}`
  - [ ] verify that `grep "^version = \"${RELEASE_VER}\"$" Cargo.toml` produces output
  - [ ] `git push ${UPSTREAM_REMOTE} v${RELEASE_VER}`
  - [ ] `cargo publish`

- assemble vendor archive:
  - [ ] `cargo vendor target/vendor`
  - [ ] `rm -r target/vendor/winapi*gnu*/lib/*.a`
  - [ ] `tar -czf target/coreos-installer-${RELEASE_VER}-vendor.tar.gz -C target vendor`

- publish this release on GitHub:
  - [ ] find the new tag in the [GitHub tag list](https://github.com/coreos/coreos-installer/tags), click the triple dots menu, and create a release for it
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
  - [ ] `git checkout main`
  - [ ] `git pull ${UPSTREAM_REMOTE} main`
  - [ ] `git push ${UPSTREAM_REMOTE} :release-${RELEASE_VER}`
  - [ ] `git branch -d release-${RELEASE_VER}`

- Fedora packaging:
  - [ ] update the `rust-coreos-installer` spec file in [Fedora](https://src.fedoraproject.org/rpms/rust-coreos-installer)
    - bump the `Version`
    - switch the `Release` back to `1%{?dist}`
    - remove any patches obsoleted by the new release
    - update changelog
  - [ ] run `spectool -g -S rust-coreos-installer.spec`
  - [ ] run `kinit your_fas_account@FEDORAPROJECT.ORG`
  - [ ] run `fedpkg new-sources <crate-name> <vendor-tarball-name> <coreos-installer-dracut-tarball-name>`
  - [ ] PR the changes in [Fedora](https://src.fedoraproject.org/rpms/rust-coreos-installer)
  - [ ] once the PR merges to rawhide, merge rawhide into the other relevant branches (e.g. f35) then push those, for example:
    ```bash
    git checkout rawhide
    git pull --ff-only
    git checkout f35
    git merge --ff-only rawhide
    git push origin f35
    ```
  - [ ] on each of those branches run `fedpkg build`
  - [ ] once the builds have finished, submit them to [bodhi](https://bodhi.fedoraproject.org/updates/new), filling in:
    - `rust-coreos-installer` for `Packages`
    - selecting the build(s) that just completed, except for the rawhide one (which gets submitted automatically)
    - writing brief release notes like "New upstream release; see release notes at `link to GitHub release`"
    - leave `Update name` blank
    - `Type`, `Severity` and `Suggestion` can be left as `unspecified` unless it is a security release. In that case select `security` with the appropriate severity.
    - `Stable karma` and `Unstable` karma can be set to `2` and `-1`, respectively.
  - [ ] [submit a fast-track](https://github.com/coreos/fedora-coreos-config/actions/workflows/add-override.yml) for FCOS testing-devel
  - [ ] [submit a fast-track](https://github.com/coreos/fedora-coreos-config/actions/workflows/add-override.yml) for FCOS next-devel if it is [open](https://github.com/coreos/fedora-coreos-pipeline/blob/main/next-devel/README.md)

- RHCOS packaging:
  - [ ] update the `coreos-installer` spec file
    - bump the `Version`
    - switch the `Release` back to `1%{?dist}`
    - remove any patches obsoleted by the new release
    - update changelog
  - [ ] run `spectool -g -S coreos-installer.spec`
  - [ ] run `kinit your_account@REDHAT.COM`
  - [ ] run `rhpkg new-sources <crate-name> <vendor-tarball-name> <coreos-installer-dracut-tarball-name>`
  - [ ] PR the changes
  - [ ] get the PR reviewed and merge it
  - [ ] update your local repo and run `rhpkg build`
  - [ ] file ticket similar to [this one](https://issues.redhat.com/browse/ART-3772) to sync the new version to mirror.openshift.com

CentOS Stream 9 packaging:
  - [ ] to be written

[cargo-release]: https://github.com/sunng87/cargo-release
[rustup]: https://rustup.rs/
[crates-io]: https://crates.io/
[GPG setup]: https://docs.github.com/en/github/authenticating-to-github/managing-commit-signature-verification
