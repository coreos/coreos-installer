---
name: release-prep
description: Prepare a coreos-installer pre-release PR with dependency updates and release notes
---

# Release Prep

## What it does

1. Creates a `pre-release-{VERSION}` branch from main
2. Shows `Cargo.toml` diff since last release for version bound review
3. Runs `cargo update` and commits the lockfile change
4. Drafts release notes by analyzing commits since the last release
5. Formats the release notes into the proper section structure in `docs/release-notes.md`
6. Commits the release notes
7. Optionally pushes the branch and opens a PR

## Prerequisites

- Git repository is `coreos-installer`
- On `main` branch (or will checkout to it)
- `cargo` is available
- `docs/release-notes.md` exists with an "Upcoming" unreleased section at the top

## Usage

```bash
# Prepare release for a specific version
/release-prep 0.27.0

# Prepare release, automatically determining the next version
/release-prep
```

## Workflow

### Step 1: Determine the release version

If the user provided a version number, use that. Otherwise, read the current version from the top of `docs/release-notes.md` — it will have a line like:

```
## Upcoming coreos-installer X.Y.Z (unreleased)
```

Extract `X.Y.Z` as the `RELEASE_VER`.

Validate:
- Version follows semver format (X.Y.Z)
- The "Upcoming" section in `docs/release-notes.md` matches this version

```bash
# Extract version from release notes
head -10 docs/release-notes.md
```

### Step 2: Ensure we're on a clean, up-to-date main

```bash
# Check current branch and status
git status

# If not on main, ask user before switching
git checkout main
git pull origin main
```

If there are uncommitted changes, STOP and warn the user.

### Step 3: Create the pre-release branch

```bash
git checkout -b pre-release-${RELEASE_VER}
```

### Step 4: Check Cargo.toml for version bound changes

Show the diff of `Cargo.toml` since the last release tag so the user can review for unintended version bound increases:

```bash
git diff $(git describe --abbrev=0) Cargo.toml
```

Present this diff to the user and ask if everything looks correct. If the user identifies issues, STOP and let them fix manually.

### Step 5: Update dependencies

```bash
cargo update
```

Then commit the lockfile:

```bash
git add Cargo.lock && git commit -m "cargo: update dependencies"
```

If `Cargo.toml` was also modified (e.g., resolver changes), include it:

```bash
git add Cargo.lock Cargo.toml && git commit -m "cargo: update dependencies"
```

### Step 6: Analyze commits for release notes

Get all commits since the last release tag, excluding merge commits and automated Sync/dependabot commits:

```bash
# Find the last release tag
git describe --abbrev=0

# List commits since then
git log --oneline --no-merges $(git describe --abbrev=0)..HEAD
```

Categorize each commit into one of four buckets based on these rules:

**Major changes** — User-facing features or significant behavior changes:
- Signing key additions/removals (`signing-keys:`)
- New commands or subcommands
- Changes to install behavior that affect users
- Commits whose messages mention user-visible features

**Minor changes** — Small user-facing improvements or fixes:
- Bug fixes (`fix`, `Fix`)
- Small UX improvements (formatting, messages)
- Documentation changes in user-facing docs (`docs:` but not internal)
- Commits with `install:` prefix that are small fixes

**Internal changes** — Changes that don't affect users directly:
- CI/workflow changes (`ci:`, `workflows/`, `.cci.`)
- Test additions (`test`, `tests/`)
- Refactoring (`tree:`, `clippy`)
- Internal component changes (`rootmap:`, `osmet:`, `blockdev:`, `zipl:`)
- Build system changes that don't affect packaging

**Packaging changes** — Dependency and build configuration changes:
- MSRV bumps (`bump MSRV`, `rust-version`)
- Dockerfile updates (`dockerfile:`)
- Dependency version requirement changes in Cargo.toml

**Skip entirely** — Do not include in release notes:
- `Sync repo templates` commits
- `build(deps): bump` commits (Dependabot auto-updates)
- The `cargo: update dependencies` commit we just created
- Merge commits
- `release-notes:` commits (meta, already captured)

For each non-skipped commit, write a concise release note bullet point. Use the commit message as a starting point but rewrite for clarity from a user's perspective. Follow the style of existing release notes entries — see examples below.

### Release notes style guide

Based on historical entries in `docs/release-notes.md`:

- Start with a lowercase verb or component prefix: `install:`, `iso:`, `rootmap:`
- Use backticks for CLI flags, commands, paths, and crate names
- Be concise — one line per item
- Use the component prefix from the commit message when relevant
- For signing keys: `Add Fedora {N} signing key; drop Fedora {M} signing key`
- For MSRV: `Bump minimum supported Rust version (MSRV) to {version}`
- For container updates: `Update container to Fedora {N}`
- For dependency bumps: `Require \`{crate}\` >= {version}` (only when Cargo.toml lower bounds changed)

**Examples from past releases:**

```
Major changes:
- Add Fedora 45 signing key; drop Fedora 42 signing key

Minor changes:
- install: Don't require the network by default
- Restore formatting of progress reporting to pre 0.24.0 behavior.

Internal changes:
- install: Simplify firstboot-args handling in config file expansion
- s390x: Use options and logic compatible with both C-based `genprotimg` and Rust-based `pvimg`
- Add initial TMT tests and a new workflow to execute tests on PRs

Packaging changes:
- Bump minimum supported Rust version (MSRV) to 1.85.0
- Update container to Fedora 42
- Updated core dependencies, including a major upgrade to `clap` from v3 to v4 and new versions of `nix`, `pnet`, and `sha2`.
```

### Step 7: Draft the release notes

Present the drafted release notes to the user organized by category:

```
Here are the drafted release notes for {RELEASE_VER}:

Major changes:
- {items}

Minor changes:
- {items}

Internal changes:
- {items}

Packaging changes:
- {items}

Please review and let me know if any changes are needed.
```

Wait for user confirmation or edits before proceeding.

### Step 8: Update `docs/release-notes.md`

Once the user approves the release notes, edit `docs/release-notes.md`:

1. **Change the "Upcoming" header** from:
   ```
   ## Upcoming coreos-installer {RELEASE_VER} (unreleased)
   ```
   to:
   ```
   ## Upcoming coreos-installer {NEXT_VER} (unreleased)
   ```
   where `NEXT_VER` is the next minor version (increment Y in X.Y.Z).

2. **Insert a new empty section** for the next unreleased version right after `# Release notes`:
   ```markdown
   ## Upcoming coreos-installer {NEXT_VER} (unreleased)

   Major changes:


   Minor changes:


   Internal changes:


   Packaging changes:

   ```

3. **Update the current release header** to include today's date:
   ```
   ## coreos-installer {RELEASE_VER} ({YYYY-MM-DD})
   ```
   Note: Remove "Upcoming" prefix and add the date.

4. **Fill in the release notes content** under each category heading. If a category has no entries, leave it empty (blank line after the heading). Make sure any existing content that was already written in the unreleased section (e.g., signing key entries added earlier) is preserved and merged with the new content.

### Step 9: Commit the release notes

```bash
git add docs/release-notes.md
git commit -m "docs/release-notes: update for release ${RELEASE_VER}"
```

### Step 10: Show summary and next steps

Present a summary to the user:

```
Pre-release prep complete for coreos-installer {RELEASE_VER}!

Branch: pre-release-{RELEASE_VER}
Commits:
  1. cargo: update dependencies
  2. docs/release-notes: update for release {RELEASE_VER}

Next steps from the release checklist:
  1. Push and open a PR:
     git push origin pre-release-{RELEASE_VER}
     Then open a PR for review.

  2. After PR merges, continue with the release checklist:
     - git checkout main && git pull origin main
     - cargo vendor-filterer target/vendor
     - cargo test --all-features --config 'source.crates-io.replace-with="vv"' --config 'source.vv.directory="target/vendor"'
     - cargo clean && git clean -fd
     - git checkout -b release-{RELEASE_VER}
     - cargo release --execute {RELEASE_VER}

  Full checklist: https://github.com/coreos/coreos-installer/issues/new?labels=release&template=release-checklist.md
```

Ask the user if they want to push the branch and open the PR now.

### Step 11: Optionally push and open PR

If the user confirms:

```bash
git push origin pre-release-${RELEASE_VER}
```

Then use `gh pr create`:

```bash
gh pr create --title "Pre-release: coreos-installer ${RELEASE_VER}" --body "$(cat <<'EOF'
## Pre-release prep for coreos-installer {RELEASE_VER}

### Changes
- Updated dependencies (`cargo update`)
- Updated release notes for {RELEASE_VER}

### Release checklist
After this PR merges, continue with the [release checklist](https://github.com/coreos/coreos-installer/issues/new?labels=release&template=release-checklist.md).
EOF
)"
```

## Checklist Coverage

This skill automates these items from the release checklist:

- [x] `git checkout -b pre-release-${RELEASE_VER}` — branch creation
- [x] `git diff $(git describe --abbrev=0) Cargo.toml` — version bound check
- [x] `cargo update` + commit — dependency update
- [x] Write release notes in `docs/release-notes.md` — drafts from git history
- [x] Commit release notes — `docs/release-notes: update for release ${RELEASE_VER}`
- [x] Push branch and open PR (optional)

## What's NOT covered

- [ ] `cargo release --execute` — requires interactive GPG signing
- [ ] `cargo publish` — requires crates.io auth
- [ ] `cargo vendor-filterer` / `cargo test` — post-merge verification
- [ ] GitHub release creation — requires uploading vendor archive
- [ ] Quay.io tag update — web UI only
- [ ] Fedora/CentOS packaging — separate repos and auth
- [ ] PR review and merge — human process

## References

- Release checklist template: `.github/ISSUE_TEMPLATE/release-checklist.md`
- Release notes: `docs/release-notes.md`
- Development docs: `DEVELOPMENT.md` (references release process)
- New release ticket: `https://github.com/coreos/coreos-installer/issues/new?labels=release&template=release-checklist.md`
