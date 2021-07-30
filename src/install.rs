// Copyright 2019 CoreOS, Inc.
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

use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use nix::mount;
use regex::Regex;
use std::fs::{
    copy as fscopy, create_dir_all, read_dir, set_permissions, File, OpenOptions, Permissions,
};
use std::io::{copy, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::blockdev::*;
use crate::cmdline::*;
use crate::download::*;
use crate::io::*;
#[cfg(target_arch = "s390x")]
use crate::s390x;
use crate::source::*;

pub fn install(config: &InstallConfig) -> Result<()> {
    // set up image source
    // we only support installing from a single artifact
    let mut sources = config.location.sources()?;
    let mut source = sources.pop().context("no artifacts found")?;
    if !sources.is_empty() {
        bail!("found multiple artifacts");
    }
    if source.signature.is_none() && config.location.require_signature() {
        if config.insecure {
            eprintln!("Signature not found; skipping verification as requested");
        } else {
            bail!("--insecure not specified and signature not found");
        }
    }

    #[cfg(target_arch = "s390x")]
    {
        if is_dasd(&config.device, None)? {
            if !config.save_partitions.is_empty() {
                // The user requested partition saving, but SavedPartitions
                // doesn't understand DASD VTOCs and won't find any partitions
                // to save.
                bail!("saving DASD partitions is not supported");
            }
            s390x::prepare_dasd(&config.device)?;
        }
    }

    // open output; ensure it's a block device and we have exclusive access
    let mut dest = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&config.device)
        .with_context(|| format!("opening {}", &config.device))?;
    if !dest
        .metadata()
        .with_context(|| format!("getting metadata for {}", &config.device))?
        .file_type()
        .is_block_device()
    {
        bail!("{} is not a block device", &config.device);
    }
    ensure_exclusive_access(&config.device)
        .with_context(|| format!("checking for exclusive access to {}", &config.device))?;

    // save partitions that we plan to keep
    let saved = SavedPartitions::new_from_disk(&mut dest, &config.save_partitions)
        .with_context(|| format!("saving partitions from {}", config.device))?;

    // get reference to partition table
    // For kpartx partitioning, this will conditionally call kpartx -d
    // when dropped
    let mut table = Disk::new(&config.device)?
        .get_partition_table()
        .with_context(|| format!("getting partition table for {}", &config.device))?;

    // copy and postprocess disk image
    // On failure, clear and reread the partition table to prevent the disk
    // from accidentally being used.
    dest.seek(SeekFrom::Start(0))
        .with_context(|| format!("seeking {}", config.device))?;
    if let Err(err) = write_disk(&config, &mut source, &mut dest, &mut *table, &saved) {
        // log the error so the details aren't dropped if we encounter
        // another error during cleanup
        eprintln!("\nError: {:?}\n", err);

        // clean up
        if config.preserve_on_error {
            eprintln!("Preserving partition table as requested");
            if saved.is_saved() {
                // The user asked to preserve the damaged partition table
                // for debugging.  We also have saved partitions, and those
                // may or may not be in the damaged table depending where we
                // failed.  Preserve the saved partitions by writing them to
                // a file in /tmp and telling the user about it.  Hey, it's
                // a debug flag.
                stash_saved_partitions(&mut dest, &saved)?;
            }
        } else {
            reset_partition_table(config, &mut dest, &mut *table, &saved)?;
        }

        // return a generic error so our exit status is right
        bail!("install failed");
    }

    eprintln!("Install complete.");
    Ok(())
}

fn ensure_exclusive_access(device: &str) -> Result<()> {
    let mut parts = Disk::new(device)?.get_busy_partitions()?;
    if parts.is_empty() {
        return Ok(());
    }
    parts.sort_unstable_by_key(|p| p.path.to_string());
    eprintln!("Partitions in use on {}:", device);
    for part in parts {
        if let Some(mountpoint) = part.mountpoint.as_ref() {
            eprintln!("    {} mounted on {}", part.path, mountpoint);
        }
        if part.swap {
            eprintln!("    {} is swap device", part.path);
        }
        for holder in part.get_holders()? {
            eprintln!("    {} in use by {}", part.path, holder);
        }
    }
    bail!("found busy partitions");
}

/// Copy the image source to the target disk and do all post-processing.
/// If this function fails, the caller should wipe the partition table
/// to ensure the user doesn't boot from a partially-written disk.
fn write_disk(
    config: &InstallConfig,
    source: &mut ImageSource,
    dest: &mut File,
    table: &mut dyn PartTable,
    saved: &SavedPartitions,
) -> Result<()> {
    // Get sector size of destination, for comparing with image
    let sector_size = get_sector_size(dest)?;

    // copy the image
    #[allow(clippy::match_bool, clippy::match_single_binding)]
    let image_copy = match is_dasd(&config.device, Some(dest))? {
        #[cfg(target_arch = "s390x")]
        true => s390x::image_copy_s390x,
        _ => image_copy_default,
    };
    write_image(
        source,
        dest,
        Path::new(&config.device),
        image_copy,
        true,
        Some(&saved),
        Some(sector_size),
    )?;
    table.reread()?;

    // postprocess
    if config.ignition.is_some()
        || config.firstboot_kargs.is_some()
        || !config.append_kargs.is_empty()
        || !config.delete_kargs.is_empty()
        || config.platform.is_some()
        || config.network_config.is_some()
        || cfg!(target_arch = "s390x")
    {
        let mount = Disk::new(&config.device)?.mount_partition_by_label(
            "boot",
            false,
            mount::MsFlags::empty(),
        )?;
        if let Some(ignition) = config.ignition.as_ref() {
            write_ignition(mount.mountpoint(), &config.ignition_hash, ignition)
                .context("writing Ignition configuration")?;
        }
        if let Some(firstboot_kargs) = config.firstboot_kargs.as_ref() {
            write_firstboot_kargs(mount.mountpoint(), firstboot_kargs)
                .context("writing firstboot kargs")?;
        }
        if !config.append_kargs.is_empty() || !config.delete_kargs.is_empty() {
            eprintln!("Modifying kernel arguments");

            visit_bls_entry_options(mount.mountpoint(), |orig_options: &str| {
                bls_entry_options_delete_and_append_kargs(
                    orig_options,
                    config.delete_kargs.as_slice(),
                    config.append_kargs.as_slice(),
                    &[],
                )
            })
            .context("deleting and appending kargs")?;
        }
        if let Some(platform) = config.platform.as_ref() {
            write_platform(mount.mountpoint(), platform).context("writing platform ID")?;
        }
        if let Some(network_config) = config.network_config.as_ref() {
            copy_network_config(mount.mountpoint(), network_config)?;
        }
        #[cfg(target_arch = "s390x")]
        {
            s390x::zipl(mount.mountpoint())?;
            s390x::chreipl(&config.device)?;
        }
    }

    // detect any latent write errors
    dest.sync_all().context("syncing data to disk")?;

    Ok(())
}

/// Write the Ignition config.
fn write_ignition(
    mountpoint: &Path,
    digest_in: &Option<IgnitionHash>,
    mut config_in: &File,
) -> Result<()> {
    eprintln!("Writing Ignition config");

    // Verify configuration digest, if any.
    if let Some(ref digest) = digest_in {
        digest
            .validate(&mut config_in)
            .context("failed to validate Ignition configuration digest")?;
        config_in
            .seek(SeekFrom::Start(0))
            .context("rewinding Ignition configuration file")?;
    };

    // make parent directory
    let mut config_dest = mountpoint.to_path_buf();
    config_dest.push("ignition");
    if !config_dest.is_dir() {
        create_dir_all(&config_dest).with_context(|| {
            format!(
                "creating Ignition config directory {}",
                config_dest.display()
            )
        })?;
        // Ignition data may contain secrets; restrict to root
        set_permissions(&config_dest, Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "setting file mode for Ignition directory {}",
                config_dest.display()
            )
        })?;
    }

    // do the copy
    config_dest.push("config.ign");
    let mut config_out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_dest)
        .with_context(|| {
            format!(
                "opening destination Ignition config {}",
                config_dest.display()
            )
        })?;
    // Ignition config may contain secrets; restrict to root
    set_permissions(&config_dest, Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "setting file mode for destination Ignition config {}",
            config_dest.display()
        )
    })?;
    copy(&mut config_in, &mut config_out).context("writing Ignition config")?;

    Ok(())
}

/// Write first-boot kernel arguments.
fn write_firstboot_kargs(mountpoint: &Path, args: &str) -> Result<()> {
    eprintln!("Writing first-boot kernel arguments");

    // write the arguments
    let mut config_dest = mountpoint.to_path_buf();
    config_dest.push("ignition.firstboot");
    // if the file doesn't already exist, fail, since our assumptions
    // are wrong
    let mut config_out = OpenOptions::new()
        .append(true)
        .open(&config_dest)
        .with_context(|| format!("opening first-boot file {}", config_dest.display()))?;
    let contents = format!("set ignition_network_kcmdline=\"{}\"\n", args);
    config_out
        .write_all(contents.as_bytes())
        .context("writing first-boot kernel arguments")?;

    Ok(())
}

/// To be used with `visit_bls_entry_options()`. Modifies the BLS config as instructed by
/// `delete_args` and `append_args`.
pub fn bls_entry_options_delete_and_append_kargs(
    orig_options: &str,
    delete_args: &[String],
    append_args: &[String],
    append_args_if_missing: &[String],
) -> Result<Option<String>> {
    if delete_args.is_empty() && append_args.is_empty() && append_args_if_missing.is_empty() {
        return Ok(None);
    }
    Ok(Some(modify_kargs(
        orig_options,
        append_args,
        append_args_if_missing,
        &[],
        delete_args,
    )?))
}

// XXX: Need a proper parser here and share it with afterburn. The approach we use here
// is to just do a dumb substring search and replace. This is naive (e.g. doesn't
// handle occurrences in quoted args) but will work for now (one thing that saves us is
// that we're acting on our baked configs, which have straight-forward kargs).
pub fn modify_kargs(
    current_kargs: &str,
    kargs_append: &[String],
    kargs_append_if_missing: &[String],
    kargs_replace: &[String],
    kargs_delete: &[String],
) -> Result<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^([^=]+)=([^=]+)=([^=]+)$").unwrap();
    }
    let mut new_kargs: String = format!(" {} ", current_kargs);
    for karg in kargs_delete {
        let s = format!(" {} ", karg.trim());
        new_kargs = new_kargs.replace(&s, " ");
    }
    for karg in kargs_append {
        new_kargs.push_str(karg.trim());
        new_kargs.push(' ');
    }
    for karg in kargs_append_if_missing {
        let karg = karg.trim();
        let s = format!(" {} ", karg);
        if !new_kargs.contains(&s) {
            new_kargs.push_str(karg);
            new_kargs.push(' ');
        }
    }
    for karg in kargs_replace {
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

/// Override the platform ID.
fn write_platform(mountpoint: &Path, platform: &str) -> Result<()> {
    // early return if setting the platform to the default value, since
    // otherwise we'll think we failed to set it
    if platform == "metal" {
        return Ok(());
    }

    eprintln!("Setting platform to {}", platform);
    visit_bls_entry_options(mountpoint, |orig_options: &str| {
        bls_entry_options_write_platform(orig_options, platform)
    })?;

    Ok(())
}

/// To be used with `visit_bls_entry_options()`. Modifies the BLS config, only changing the
/// `ignition.platform.id`. This assumes that we will only install from metal images and that the
/// bootloader configs will always set ignition.platform.id.  Fail if those assumptions change.
/// This is deliberately simplistic.
fn bls_entry_options_write_platform(orig_options: &str, platform: &str) -> Result<Option<String>> {
    let new_options = orig_options.replace(
        "ignition.platform.id=metal",
        &format!("ignition.platform.id={}", platform),
    );
    if orig_options == new_options {
        bail!("Couldn't locate platform ID");
    }
    Ok(Some(new_options))
}

/// Calls a function on the latest (default) BLS entry and optionally updates it if the function
/// returns new content. Errors out if no BLS entry was found.
///
/// Note that on s390x, this does not handle the call to `zipl`. We expect it to be done at a
/// higher level if needed for batching purposes.
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
/// `options` field was found.
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

/// Copy networking config if asked to do so
fn copy_network_config(mountpoint: &Path, net_config_src: &str) -> Result<()> {
    eprintln!("Copying networking configuration from {}", net_config_src);

    // get the path to the destination directory
    let net_config_dest = mountpoint.join("coreos-firstboot-network");

    // make the directory if it doesn't exist
    create_dir_all(&net_config_dest).with_context(|| {
        format!(
            "creating destination networking config directory {}",
            net_config_dest.display()
        )
    })?;

    // copy files from source to destination directories
    for entry in read_dir(&net_config_src)
        .with_context(|| format!("reading directory {}", net_config_src))?
    {
        let entry = entry.with_context(|| format!("reading directory {}", net_config_src))?;
        let srcpath = entry.path();
        let destpath = net_config_dest.join(entry.file_name());
        if srcpath.is_file() {
            eprintln!("Copying {} to installed system", srcpath.display());
            fscopy(&srcpath, &destpath).context("Copying networking config")?;
        }
    }

    Ok(())
}

/// Clear the partition table and restore saved partitions.  For use after
/// a failure.
fn reset_partition_table(
    config: &InstallConfig,
    dest: &mut File,
    table: &mut dyn PartTable,
    saved: &SavedPartitions,
) -> Result<()> {
    eprintln!("Resetting partition table");

    if is_dasd(&config.device, Some(dest))? {
        // Don't write out a GPT, since the backup GPT may overwrite
        // something we're not allowed to touch.  Just clear the first MiB
        // of disk.
        dest.seek(SeekFrom::Start(0))
            .context("seeking to start of disk")?;
        let zeroes = [0u8; 1024 * 1024];
        dest.write_all(&zeroes)
            .context("clearing primary partition table")?;
    } else {
        // Write a new GPT including any saved partitions.
        saved
            .overwrite(dest)
            .context("restoring saved partitions")?;
    }

    // Finish writeback and reread the partition table.
    dest.sync_all().context("syncing partition table to disk")?;
    table.reread()?;

    Ok(())
}

// Preserve saved partitions by writing them to a file in /tmp and reporting
// the path.
fn stash_saved_partitions(disk: &mut File, saved: &SavedPartitions) -> Result<()> {
    let mut stash = tempfile::Builder::new()
        .prefix("coreos-installer-partitions.")
        .tempfile()
        .context("creating partition stash file")?;
    let path = stash.path().to_owned();
    eprintln!("Storing saved partition entries to {}", path.display());
    let len = disk.seek(SeekFrom::End(0)).context("seeking disk")?;
    stash
        .as_file()
        .set_len(len)
        .with_context(|| format!("extending partition stash file {}", path.display()))?;
    saved
        .overwrite(stash.as_file_mut())
        .with_context(|| format!("stashing saved partitions to {}", path.display()))?;
    stash
        .keep()
        .with_context(|| format!("retaining saved partition stash in {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_id() {
        let orig_content = "ignition.platform.id=metal foo bar";
        let new_content = bls_entry_options_write_platform(orig_content, "openstack").unwrap();
        assert_eq!(
            new_content.unwrap(),
            "ignition.platform.id=openstack foo bar"
        );

        let orig_content = "foo ignition.platform.id=metal bar";
        let new_content = bls_entry_options_write_platform(orig_content, "openstack").unwrap();
        assert_eq!(
            new_content.unwrap(),
            "foo ignition.platform.id=openstack bar"
        );

        let orig_content = "foo bar ignition.platform.id=metal";
        let new_content = bls_entry_options_write_platform(orig_content, "openstack").unwrap();
        assert_eq!(
            new_content.unwrap(),
            "foo bar ignition.platform.id=openstack"
        );
    }

    #[test]
    fn test_modify_kargs() {
        let orig_kargs = "foo bar foobar";

        let delete_kargs = vec!["foo".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
        assert_eq!(new_kargs, "bar foobar");

        let delete_kargs = vec!["bar".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
        assert_eq!(new_kargs, "foo foobar");

        let delete_kargs = vec!["foobar".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
        assert_eq!(new_kargs, "foo bar");

        let delete_kargs = vec!["foo bar".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
        assert_eq!(new_kargs, "foobar");

        let delete_kargs = vec!["bar".into(), "foo".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
        assert_eq!(new_kargs, "foobar");

        let orig_kargs = "foo=val bar baz=val";

        let delete_kargs = vec!["   foo=val".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
        assert_eq!(new_kargs, "bar baz=val");

        let delete_kargs = vec!["baz=val  ".into()];
        let new_kargs = modify_kargs(orig_kargs, &[], &[], &[], &delete_kargs).unwrap();
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
        let new_kargs = modify_kargs(
            orig_kargs,
            &append_kargs,
            &append_kargs_if_missing,
            &[],
            &delete_kargs,
        )
        .unwrap();
        assert_eq!(
            new_kargs,
            "foo console=tty0 bar baz console=ttyS1,115200n8 boo"
        );

        let orig_kargs = "foo mitigations=auto,nosmt console=tty0 bar console=ttyS0,115200n8 baz";

        let append_kargs = vec!["console=ttyS1,115200n8".into()];
        let replace_kargs = vec!["mitigations=auto,nosmt=auto".into()];
        let delete_kargs = vec!["console=ttyS0,115200n8".into()];
        let new_kargs = modify_kargs(
            orig_kargs,
            &append_kargs,
            &[],
            &replace_kargs,
            &delete_kargs,
        )
        .unwrap();
        assert_eq!(
            new_kargs,
            "foo mitigations=auto console=tty0 bar baz console=ttyS1,115200n8"
        );
    }
}
