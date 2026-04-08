---
name: signing-key-rotation
description: Add a new Fedora signing key and drop the oldest one in coreos-installer
---

# Fedora Signing Key Rotation

## What it does

1. Downloads the new Fedora GPG signing key from src.fedoraproject.org
2. Appends it to `src/signing-keys.asc`
3. Updates `docs/release-notes.md` with the addition
4. Creates a commit: `signing-keys: add Fedora {N} key`
5. Removes the oldest Fedora key (N-3) from `src/signing-keys.asc`
6. Updates `docs/release-notes.md` with the removal
7. Creates a commit: `signing-keys: drop Fedora {N-3} key`

## Prerequisites

- Git repository is `coreos-installer`
- `src/signing-keys.asc` exists with current signing keys
- `docs/release-notes.md` exists with an unreleased section at the top
- Network access to download the key from src.fedoraproject.org

## Usage

```bash
# Rotate keys for a new Fedora version
/signing-key-rotation 46

# Specify a custom old version to drop (default: NEW - 3)
/signing-key-rotation 46 --drop 43

# Include a tracker issue reference for the drop commit
/signing-key-rotation 46 --tracker https://github.com/coreos/fedora-coreos-tracker/issues/XXXX
```

## Workflow

### Step 1: Parse arguments and validate

Extract the new Fedora version number from the user's invocation. If not provided, ask for it.

```
NEW_VERSION = (from user input)
OLD_VERSION = NEW_VERSION - 3  (unless --drop is specified)
TRACKER_URL = (from --tracker, optional)
```

Run these validation checks:

```bash
# Verify signing-keys.asc exists
ls src/signing-keys.asc

# Check what keys are currently present
grep "Comment:" src/signing-keys.asc

# Verify the old key exists
grep "RPM-GPG-KEY-fedora-${OLD_VERSION}-primary" src/signing-keys.asc

# Verify the new key does NOT already exist
grep "RPM-GPG-KEY-fedora-${NEW_VERSION}-primary" src/signing-keys.asc
```

If the new key already exists, STOP and inform the user.
If the old key does not exist, STOP and inform the user.

### Step 2: Download the new GPG key

Fetch the key from Fedora's package sources:

```
URL: https://src.fedoraproject.org/rpms/fedora-repos/blob/rawhide/f/RPM-GPG-KEY-fedora-{NEW_VERSION}-primary
```

Use the WebFetch tool to download the page, then extract the raw PGP key block between `-----BEGIN PGP PUBLIC KEY BLOCK-----` and `-----END PGP PUBLIC KEY BLOCK-----` (inclusive).

If the page is not found or doesn't contain a PGP key block, STOP and inform the user that the key may not exist yet.

### Step 3: Add the new key to `src/signing-keys.asc`

Using the Edit tool, append to the end of `src/signing-keys.asc`:

1. A blank line after the last `-----END PGP PUBLIC KEY BLOCK-----`
2. The complete PGP key block (from `-----BEGIN PGP PUBLIC KEY BLOCK-----` through `-----END PGP PUBLIC KEY BLOCK-----`)

Make sure the file ends with a newline.

### Step 4: Update release notes for the addition

Using the Edit tool on `docs/release-notes.md`, find the first "Major changes:" section (the unreleased version at the top) and add:

```
- Add Fedora {NEW_VERSION} signing key
```

The entry goes right after the "Major changes:" line. If there are already entries there, add it as the first bullet point. If the section is empty (just blank lines between "Major changes:" and the next heading), add it on the blank line.

### Step 5: Create the "add" commit

Do NOT commit unless the user has asked you to commit. If they have, create the commit:

```bash
git add src/signing-keys.asc docs/release-notes.md
git commit -m "signing-keys: add Fedora {NEW_VERSION} key

Fedora {NEW_VERSION - 1} branched from rawhide, so rawhide is now F{NEW_VERSION}.

Ref: https://src.fedoraproject.org/rpms/fedora-repos/blob/rawhide/f/RPM-GPG-KEY-fedora-{NEW_VERSION}-primary"
```

### Step 6: Remove the old key from `src/signing-keys.asc`

Using the Edit tool, find and remove the entire PGP key block that contains:

```
Comment: RPM-GPG-KEY-fedora-{OLD_VERSION}-primary
```

Remove from the blank line before `-----BEGIN PGP PUBLIC KEY BLOCK-----` through the `-----END PGP PUBLIC KEY BLOCK-----` line (inclusive). Also remove any trailing blank line that was part of the separator between key blocks.

### Step 7: Update release notes for the removal

Using the Edit tool on `docs/release-notes.md`, change:

```
- Add Fedora {NEW_VERSION} signing key
```

to:

```
- Add Fedora {NEW_VERSION} signing key; drop Fedora {OLD_VERSION} signing key
```

### Step 8: Create the "drop" commit

Do NOT commit unless the user has asked you to commit. If they have:

If a tracker URL was provided:

```bash
git add src/signing-keys.asc docs/release-notes.md
git commit -m "signing-keys: drop Fedora {OLD_VERSION} key

Ref: {TRACKER_URL}"
```

If no tracker URL:

```bash
git add src/signing-keys.asc docs/release-notes.md
git commit -m "signing-keys: drop Fedora {OLD_VERSION} key"
```

### Step 9: Verify

Run verification checks:

```bash
# Confirm exactly 4 key blocks (1 RHEL + 3 Fedora)
grep "Comment:" src/signing-keys.asc

# Verify the new key is present
grep "RPM-GPG-KEY-fedora-${NEW_VERSION}-primary" src/signing-keys.asc

# Verify the old key is gone
grep "RPM-GPG-KEY-fedora-${OLD_VERSION}-primary" src/signing-keys.asc
# (should return no match)

# Show the git log
git log --oneline -2
```

Optionally run `cargo check` to verify the build still works.

### Step 10: Report results

Present the user with a summary:

```
Signing key rotation complete:

  Added:   Fedora {NEW_VERSION} signing key
  Dropped: Fedora {OLD_VERSION} signing key

  Current keys in src/signing-keys.asc:
    - RPM-GPG-KEY-redhat-release
    - RPM-GPG-KEY-fedora-{OLD_VERSION + 1}-primary
    - RPM-GPG-KEY-fedora-{OLD_VERSION + 2}-primary
    - RPM-GPG-KEY-fedora-{NEW_VERSION}-primary

  Commits created:
    1. signing-keys: add Fedora {NEW_VERSION} key
    2. signing-keys: drop Fedora {OLD_VERSION} key

  Files modified:
    - src/signing-keys.asc
    - docs/release-notes.md

  Next steps:
    1. Push to a branch and open a PR
    2. Get review and merge
```

## Checklist Coverage

This skill automates the following from the manual process:

- [x] Download new Fedora GPG key
- [x] Append key to `src/signing-keys.asc`
- [x] Remove old key from `src/signing-keys.asc`
- [x] Update `docs/release-notes.md`
- [x] Create properly formatted commits
- [x] Verify key count and correctness

## What's NOT covered

- [ ] Creating the PR on GitHub (user can do this manually or ask)
- [ ] Determining the tracker issue URL (user must provide if desired)
- [ ] Running full CI (`cargo test`) - only `cargo check` is optionally run
- [ ] Pushing to a remote branch

## Example Output

```
$ /signing-key-rotation 46

Validating inputs...
  NEW_VERSION: 46
  OLD_VERSION: 43 (46 - 3)
  Current keys: redhat-release, fedora-43, fedora-44, fedora-45

Downloading Fedora 46 key from src.fedoraproject.org...
  Found PGP key block (31 lines)

Adding Fedora 46 key to src/signing-keys.asc...
  Appended key block to end of file

Updating docs/release-notes.md...
  Added "Add Fedora 46 signing key" to Major changes

Created commit: signing-keys: add Fedora 46 key

Removing Fedora 43 key from src/signing-keys.asc...
  Removed key block with Comment: RPM-GPG-KEY-fedora-43-primary

Updating docs/release-notes.md...
  Updated to "Add Fedora 46 signing key; drop Fedora 43 signing key"

Created commit: signing-keys: drop Fedora 43 key

Verification:
  Keys present: redhat-release, fedora-44, fedora-45, fedora-46
  Key count: 4 (correct)
  cargo check: passed

Signing key rotation complete!
```

## References

- Design doc: `.opencode/skills/signing-key-rotation/DESIGN.md`
- Example 1 (F45/F42): `.opencode/skills/signing-key-rotation/examples/example-1-add-f45-drop-f42.md`
- Example 2 (F43/F40): `.opencode/skills/signing-key-rotation/examples/example-2-add-f43-drop-f40.md`
- Key source: `https://src.fedoraproject.org/rpms/fedora-repos/blob/rawhide/f/RPM-GPG-KEY-fedora-{N}-primary`
- Tracker: `https://github.com/coreos/fedora-coreos-tracker/issues`
