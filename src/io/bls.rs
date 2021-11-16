// Copyright 2021 CoreOS, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Utilities for reading/writing BLS configs, including kernel arguments.

use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use regex::Regex;
use std::fs::{read_dir, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Calls a function on the latest (default) BLS entry and optionally updates it if the function
/// returns new content. Errors out if no BLS entry was found.
///
/// Note that on s390x, this does not handle the call to `zipl`. We expect it to be done at a
/// higher level if needed for batching purposes.
///
/// Returns `true` if BLS content was modified.
pub fn visit_bls_entry(
    mountpoint: &Path,
    f: impl Fn(&str) -> Result<Option<String>>,
) -> Result<bool> {
    // walk /boot/loader/entries/*.conf
    let mut config_path = mountpoint.to_path_buf();
    config_path.push("loader/entries");

    // We only want to affect the latest BLS entry (i.e. the default one). This confusingly is the
    // *last* BLS config in the directory because they are sorted by reverse order:
    // https://github.com/ostreedev/ostree/pull/1654
    //
    // Because `read_dir` doesn't guarantee any ordering, we gather all the filenames up front and
    // sort them before picking the last one.
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in read_dir(&config_path)
        .with_context(|| format!("reading directory {}", config_path.display()))?
    {
        let path = entry
            .with_context(|| format!("reading directory {}", config_path.display()))?
            .path();
        if path.extension().unwrap_or_default() != "conf" {
            continue;
        }
        entries.push(path);
    }
    entries.sort();

    let mut changed = false;
    if let Some(path) = entries.pop() {
        // slurp in the file
        let mut config = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening bootloader config {}", path.display()))?;
        let orig_contents = {
            let mut s = String::new();
            config
                .read_to_string(&mut s)
                .with_context(|| format!("reading {}", path.display()))?;
            s
        };

        let r = f(&orig_contents).with_context(|| format!("visiting {}", path.display()))?;

        if let Some(new_contents) = r {
            // write out the modified data
            config
                .seek(SeekFrom::Start(0))
                .with_context(|| format!("seeking {}", path.display()))?;
            config
                .set_len(0)
                .with_context(|| format!("truncating {}", path.display()))?;
            config
                .write(new_contents.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            changed = true;
        }
    } else {
        bail!("Found no BLS entries in {}", config_path.display());
    }

    Ok(changed)
}

/// Wrapper around `visit_bls_entry` to specifically visit just the BLS entry's `options` line and
/// optionally update it if the function returns new content. Errors out if none or more than one
/// `options` field was found. Returns `true` if BLS content was modified.
pub fn visit_bls_entry_options(
    mountpoint: &Path,
    f: impl Fn(&str) -> Result<Option<String>>,
) -> Result<bool> {
    visit_bls_entry(mountpoint, |orig_contents: &str| {
        let mut new_contents = String::with_capacity(orig_contents.len());
        let mut found_options = false;
        let mut modified = false;
        for line in orig_contents.lines() {
            if !line.starts_with("options ") {
                new_contents.push_str(line.trim_end());
            } else if found_options {
                bail!("Multiple 'options' lines found");
            } else {
                let r = f(line["options ".len()..].trim()).context("visiting options")?;
                if let Some(new_options) = r {
                    new_contents.push_str("options ");
                    new_contents.push_str(new_options.trim());
                    modified = true;
                }
                found_options = true;
            }
            new_contents.push('\n');
        }
        if !found_options {
            bail!("Couldn't locate 'options' line");
        }
        if !modified {
            Ok(None)
        } else {
            Ok(Some(new_contents))
        }
    })
}

#[derive(Default, PartialEq)]
pub struct KargsEditor {
    append: Vec<String>,
    append_if_missing: Vec<String>,
    replace: Vec<String>,
    delete: Vec<String>,
}

impl KargsEditor {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn append(&mut self, args: &[String]) -> &mut Self {
        self.append.extend_from_slice(args);
        self
    }

    pub fn append_if_missing(&mut self, args: &[String]) -> &mut Self {
        self.append_if_missing.extend_from_slice(args);
        self
    }

    pub fn replace(&mut self, args: &[String]) -> &mut Self {
        self.replace.extend_from_slice(args);
        self
    }

    pub fn delete(&mut self, args: &[String]) -> &mut Self {
        self.delete.extend_from_slice(args);
        self
    }

    // XXX: Need a proper parser here and share it with afterburn. The approach we use here
    // is to just do a dumb substring search and replace. This is naive (e.g. doesn't
    // handle occurrences in quoted args) but will work for now (one thing that saves us is
    // that we're acting on our baked configs, which have straight-forward kargs).
    pub fn apply_to(&self, current_kargs: &str) -> Result<String> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"^([^=]+)=([^=]+)=([^=]+)$").unwrap();
        }
        let mut new_kargs: String = format!(" {} ", current_kargs);
        for karg in &self.delete {
            let s = format!(" {} ", karg.trim());
            new_kargs = new_kargs.replace(&s, " ");
        }
        for karg in &self.append {
            new_kargs.push_str(karg.trim());
            new_kargs.push(' ');
        }
        for karg in &self.append_if_missing {
            let karg = karg.trim();
            let s = format!(" {} ", karg);
            if !new_kargs.contains(&s) {
                new_kargs.push_str(karg);
                new_kargs.push(' ');
            }
        }
        for karg in &self.replace {
            let caps = match RE.captures(karg) {
                Some(caps) => caps,
                None => bail!("Wrong input, format should be: KEY=OLD=NEW"),
            };
            let old = format!(" {}={} ", &caps[1], &caps[2]);
            let new = format!(" {}={} ", &caps[1], &caps[3]);
            new_kargs = new_kargs.replace(&old, &new);
        }
        Ok(new_kargs.trim().into())
    }

    /// Return None if we haven't been asked to do anything, otherwise
    /// Some(modified args).
    /// To be used with `visit_bls_entry_options()`.
    pub fn maybe_apply_to(&self, current_kargs: &str) -> Result<Option<String>> {
        if self == &Self::new() {
            Ok(None)
        } else {
            Ok(Some(self.apply_to(current_kargs)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_to() {
        let orig_kargs = "foo bar foobar";

        let delete_kargs = vec!["foo".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "bar foobar");

        let delete_kargs = vec!["bar".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "foo foobar");

        let delete_kargs = vec!["foobar".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "foo bar");

        let delete_kargs = vec!["foo bar".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "foobar");

        let delete_kargs = vec!["bar".into(), "foo".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "foobar");

        let orig_kargs = "foo=val bar baz=val";

        let delete_kargs = vec!["   foo=val".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "bar baz=val");

        let delete_kargs = vec!["baz=val  ".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(new_kargs, "foo=val bar");

        let orig_kargs = "foo mitigations=auto,nosmt console=tty0 bar console=ttyS0,115200n8 baz";

        let delete_kargs = vec![
            "mitigations=auto,nosmt".into(),
            "console=ttyS0,115200n8".into(),
        ];
        let append_kargs = vec!["console=ttyS1,115200n8  ".into()];
        let append_kargs_if_missing =
                 // base       // append_kargs dupe             // missing
            vec!["bar".into(), "console=ttyS1,115200n8".into(), "boo".into()];
        let new_kargs = KargsEditor::new()
            .delete(&delete_kargs)
            .append(&append_kargs)
            .append_if_missing(&append_kargs_if_missing)
            .apply_to(orig_kargs)
            .unwrap();
        assert_eq!(
            new_kargs,
            "foo console=tty0 bar baz console=ttyS1,115200n8 boo"
        );

        let orig_kargs = "foo mitigations=auto,nosmt console=tty0 bar console=ttyS0,115200n8 baz";

        let append_kargs = vec!["console=ttyS1,115200n8".into()];
        let replace_kargs = vec!["mitigations=auto,nosmt=auto".into()];
        let delete_kargs = vec!["console=ttyS0,115200n8".into()];
        let new_kargs = KargsEditor::new()
            .append(&append_kargs)
            .replace(&replace_kargs)
            .delete(&delete_kargs)
            .apply_to(&orig_kargs)
            .unwrap();
        assert_eq!(
            new_kargs,
            "foo mitigations=auto console=tty0 bar baz console=ttyS1,115200n8"
        );
    }

    #[test]
    fn test_maybe_apply_to() {
        // no arguments
        assert!(KargsEditor::new()
            .maybe_apply_to("foo bar foobar")
            .unwrap()
            .is_none());

        // empty arguments
        assert!(KargsEditor::new()
            .append(&[])
            .delete(&[])
            .maybe_apply_to("foo bar foobar")
            .unwrap()
            .is_none());

        // arguments that aren't relevant
        let new_kargs = KargsEditor::new()
            .delete(&["baz".into()])
            .maybe_apply_to("foo bar foobar")
            .unwrap()
            .unwrap();
        assert_eq!(new_kargs, "foo bar foobar");
    }
}
