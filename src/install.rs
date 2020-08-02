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

use error_chain::{bail, ChainedError};
use nix::mount;
use std::fs::{canonicalize, copy as fscopy, create_dir_all, read_dir, File, OpenOptions};
use std::io::{copy, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use crate::blockdev::*;
use crate::cmdline::*;
use crate::download::*;
use crate::errors::*;
use crate::io::*;
#[cfg(target_arch = "s390x")]
use crate::s390x;
use crate::source::*;

pub fn install(config: &InstallConfig) -> Result<()> {
    // set up image source
    // we only support installing from a single artifact
    let mut sources = config.location.sources()?;
    let mut source = sources.pop().chain_err(|| "no artifacts found")?;
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
        if is_dasd(config)? {
            if !config.save_partitions.is_empty() {
                // The user requested partition saving, but SavedPartitions
                // doesn't understand DASD VTOCs and won't find any partitions
                // to save.
                bail!("saving DASD partitions is not supported");
            }
            s390x::prepare_dasd(&config)?;
        }
    }

    // open output; ensure it's a block device and we have exclusive access
    let mut dest = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&config.device)
        .chain_err(|| format!("opening {}", &config.device))?;
    if !dest
        .metadata()
        .chain_err(|| format!("getting metadata for {}", &config.device))?
        .file_type()
        .is_block_device()
    {
        bail!("{} is not a block device", &config.device);
    }
    ensure_exclusive_access(&config.device)
        .chain_err(|| format!("checking for exclusive access to {}", &config.device))?;

    // save partitions that we plan to keep
    let saved = SavedPartitions::new(&config.device, &config.save_partitions)?;

    // get reference to partition table
    // For kpartx partitioning, this will conditionally call kpartx -d
    // when dropped
    let mut table = Disk::new(&config.device)
        .get_partition_table()
        .chain_err(|| format!("getting partition table for {}", &config.device))?;

    // copy and postprocess disk image
    // On failure, clear and reread the partition table to prevent the disk
    // from accidentally being used.
    if let Err(err) = write_disk(&config, &mut source, &mut dest, &mut *table, &saved) {
        // log the error so the details aren't dropped if we encounter
        // another error during cleanup
        eprint!("{}", ChainedError::display_chain(&err));

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
                stash_saved_partitions(&saved)?;
            }
        } else {
            clear_partition_table(&mut dest, &mut *table)?;
            saved
                .write(&config.device)
                .chain_err(|| "restoring additional partitions")?;
        }

        // return a generic error so our exit status is right
        bail!("install failed");
    }

    eprintln!("Install complete.");
    Ok(())
}

fn ensure_exclusive_access(device: &str) -> Result<()> {
    let mut parts = Disk::new(device).get_busy_partitions()?;
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
    let image_copy = match is_dasd(config)? {
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
        saved
            .get_offset()?
            .map(|(offset, desc)| (offset, format!("collision with {}", desc))),
        Some(sector_size),
    )?;

    // restore saved partitions, if any, and reread table
    saved
        .write(&config.device)
        .chain_err(|| "restoring saved partitions")?;
    table.reread()?;

    // postprocess
    if config.ignition.is_some()
        || config.firstboot_kargs.is_some()
        || config.append_kargs.is_some()
        || config.delete_kargs.is_some()
        || config.platform.is_some()
        || config.network_config.is_some()
        || cfg!(target_arch = "s390x")
    {
        let mount = Disk::new(&config.device).mount_partition_by_label(
            "boot",
            false,
            mount::MsFlags::empty(),
        )?;
        if let Some(ignition) = config.ignition.as_ref() {
            write_ignition(mount.mountpoint(), &config.ignition_hash, ignition)
                .chain_err(|| "writing Ignition configuration")?;
        }
        if let Some(firstboot_kargs) = config.firstboot_kargs.as_ref() {
            write_firstboot_kargs(mount.mountpoint(), firstboot_kargs)
                .chain_err(|| "writing firstboot kargs")?;
        }
        if config.append_kargs.is_some() || config.delete_kargs.is_some() {
            eprintln!("Modifying kernel arguments");

            edit_bls_entries(mount.mountpoint(), |orig_contents: &str| {
                bls_entry_delete_and_append_kargs(
                    orig_contents,
                    config.delete_kargs.as_ref(),
                    config.append_kargs.as_ref(),
                )
            })
            .chain_err(|| "deleting and appending kargs")?;
        }
        if let Some(platform) = config.platform.as_ref() {
            write_platform(mount.mountpoint(), platform).chain_err(|| "writing platform ID")?;
        }
        if let Some(network_config) = config.network_config.as_ref() {
            copy_network_config(mount.mountpoint(), network_config)?;
        }
        #[cfg(target_arch = "s390x")]
        s390x::install_bootloader(mount.mountpoint(), &config.device)?;
    }

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
            .chain_err(|| "failed to validate Ignition configuration digest")?;
        config_in
            .seek(SeekFrom::Start(0))
            .chain_err(|| "rewinding Ignition configuration file")?;
    };

    // make parent directory
    let mut config_dest = mountpoint.to_path_buf();
    config_dest.push("ignition");
    create_dir_all(&config_dest).chain_err(|| "creating Ignition config directory")?;

    // do the copy
    config_dest.push("config.ign");
    let mut config_out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config_dest)
        .chain_err(|| {
            format!(
                "opening destination Ignition config {}",
                config_dest.display()
            )
        })?;
    copy(&mut config_in, &mut config_out).chain_err(|| "writing Ignition config")?;

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
        .chain_err(|| format!("opening first-boot file {}", config_dest.display()))?;
    let contents = format!("set ignition_network_kcmdline=\"{}\"\n", args);
    config_out
        .write_all(contents.as_bytes())
        .chain_err(|| "writing first-boot kernel arguments")?;

    Ok(())
}

// This is split out so that we can unit test it.
pub fn bls_entry_delete_and_append_kargs(
    orig_contents: &str,
    delete_args: Option<&Vec<String>>,
    append_args: Option<&Vec<String>>,
) -> Result<String> {
    let mut new_contents = String::with_capacity(orig_contents.len());
    let mut found_options = false;
    for line in orig_contents.lines() {
        if !line.starts_with("options ") {
            new_contents.push_str(line.trim_end());
        } else if found_options {
            bail!("Multiple 'options' lines found");
        } else {
            // XXX: Need a proper parser here and share it with afterburn. The approach we use here
            // is to just do a dumb substring search and replace. This is naive (e.g. doesn't
            // handle occurrences in quoted args) but will work for now (one thing that saves us is
            // that we're acting on our baked configs, which have straight-forward kargs).
            new_contents.push_str("options ");
            let mut line: String = add_whitespaces(&line["options ".len()..]);
            if let Some(args) = delete_args {
                for arg in args {
                    let arg = add_whitespaces(&arg);
                    line = line.replace(&arg, " ");
                }
            }
            new_contents.push_str(line.trim_start().trim_end());
            if let Some(args) = append_args {
                for arg in args {
                    new_contents.push(' ');
                    new_contents.push_str(&arg);
                }
            }
            found_options = true;
        }
        new_contents.push('\n');
    }
    if !found_options {
        bail!("Couldn't locate 'options' line");
    }
    Ok(new_contents)
}

fn add_whitespaces(s: &str) -> String {
    let mut r: String = s.into();
    r.insert(0, ' ');
    r.push(' ');
    r
}

/// Override the platform ID.
fn write_platform(mountpoint: &Path, platform: &str) -> Result<()> {
    // early return if setting the platform to the default value, since
    // otherwise we'll think we failed to set it
    if platform == "metal" {
        return Ok(());
    }

    eprintln!("Setting platform to {}", platform);
    edit_bls_entries(mountpoint, |orig_contents: &str| {
        bls_entry_write_platform(orig_contents, platform)
    })?;

    Ok(())
}

/// Modifies the BLS config, only changing the `ignition.platform.id`. This assumes that we will
/// only install from metal images and that the bootloader configs will always set
/// ignition.platform.id.  Fail if those assumptions change.  This is deliberately simplistic.
fn bls_entry_write_platform(orig_contents: &str, platform: &str) -> Result<String> {
    let new_contents = orig_contents.replace(
        "ignition.platform.id=metal",
        &format!("ignition.platform.id={}", platform),
    );
    if orig_contents == new_contents {
        bail!("Couldn't locate platform ID");
    }
    Ok(new_contents)
}

/// Apply a transforming function on each BLS entry found.
pub fn edit_bls_entries(mountpoint: &Path, f: impl Fn(&str) -> Result<String>) -> Result<()> {
    // walk /boot/loader/entries/*.conf
    let mut config_path = mountpoint.to_path_buf();
    config_path.push("loader/entries");
    for entry in read_dir(&config_path)
        .chain_err(|| format!("reading directory {}", config_path.display()))?
    {
        let path = entry
            .chain_err(|| format!("reading directory {}", config_path.display()))?
            .path();
        if path.extension().unwrap_or_default() == "conf" {
            // slurp in the file
            let mut config = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .chain_err(|| format!("opening bootloader config {}", path.display()))?;
            let orig_contents = {
                let mut s = String::new();
                config
                    .read_to_string(&mut s)
                    .chain_err(|| format!("reading {}", path.display()))?;
                s
            };

            let new_contents =
                f(&orig_contents).chain_err(|| format!("modifying {}", path.display()))?;

            // write out the modified data
            config
                .seek(SeekFrom::Start(0))
                .chain_err(|| format!("seeking {}", path.display()))?;
            config
                .set_len(0)
                .chain_err(|| format!("truncating {}", path.display()))?;
            config
                .write(new_contents.as_bytes())
                .chain_err(|| format!("writing {}", path.display()))?;
        }
    }

    Ok(())
}

/// Copy networking config if asked to do so
fn copy_network_config(mountpoint: &Path, net_config_src: &str) -> Result<()> {
    eprintln!("Copying networking configuration from {}", net_config_src);

    // get the path to the destination directory
    let net_config_dest = mountpoint.join("coreos-firstboot-network");

    // make the directory if it doesn't exist
    create_dir_all(&net_config_dest).chain_err(|| {
        format!(
            "creating destination networking config directory {}",
            net_config_dest.display()
        )
    })?;

    // copy files from source to destination directories
    for entry in
        read_dir(&net_config_src).chain_err(|| format!("reading directory {}", net_config_src))?
    {
        let entry = entry.chain_err(|| format!("reading directory {}", net_config_src))?;
        let srcpath = entry.path();
        let destpath = net_config_dest.join(entry.file_name());
        if srcpath.is_file() {
            eprintln!("Copying {} to installed system", srcpath.display());
            fscopy(&srcpath, &destpath).chain_err(|| "Copying networking config")?;
        }
    }

    Ok(())
}

/// Clear the partition table.  For use after a failure.
fn clear_partition_table(dest: &mut File, table: &mut dyn PartTable) -> Result<()> {
    eprintln!("Clearing partition table");
    dest.seek(SeekFrom::Start(0))
        .chain_err(|| "seeking to start of disk")?;
    let zeroes = [0u8; 1024 * 1024];
    dest.write_all(&zeroes)
        .chain_err(|| "clearing primary partition table")?;

    // Now the backup GPT, which is in the last LBA.  If there is one, we
    // should clear it, since it might have stale partition info.
    // Constraints:
    //   - Never overwrite partition contents.
    //   - If we're on a GPT platform, we have the right to overwrite at
    //     least the last 4 KiB of disk.  On 4Kn drives, that's the backup
    //     GPT.  On 512-byte drives, there is at least 16 KiB of
    //     non-partitionable space before the backup GPT, so we're still safe.
    //     This is true even if we're writing to a legacy MBR disk, because
    //     by doing so, the user already gave the OS permission to write the
    //     backup GPT on first boot.
    //   - We can't assume that the backup GPT corresponds to the disk
    //     sector size because of possible user error.  We probably can't
    //     even assume there aren't backup GPTs for both sector sizes.
    //   - If we're not on a GPT system (s390x DASD), we can't overwrite the
    //     end of the disk.
    // We could probably get away with clearing the last 4 KiB if !DASD, but
    // be a bit more conservative: probe for _any_ GPT signature and, if
    // found, clear the last 4 KiB.
    let mut buf = [0u8; 4096];
    dest.seek(SeekFrom::End(-(buf.len() as i64)))
        .chain_err(|| "seeking to end of disk")?;
    dest.read_exact(&mut buf)
        .chain_err(|| "reading end of disk")?;
    if detect_formatted_sector_size_end(&buf).is_some() {
        dest.seek(SeekFrom::End(-(buf.len() as i64)))
            .chain_err(|| "seeking to end of disk")?;
        dest.write_all(&zeroes[..buf.len()])
            .chain_err(|| "clearing backup partition table")?;
    }

    dest.flush()
        .chain_err(|| "flushing partition table to disk")?;
    dest.sync_all()
        .chain_err(|| "syncing partition table to disk")?;
    table.reread()?;
    Ok(())
}

// Preserve saved partitions by writing them to a file in /tmp and reporting
// the path.
fn stash_saved_partitions(saved: &SavedPartitions) -> Result<()> {
    let stash = tempfile::Builder::new()
        .prefix("coreos-installer-partitions.")
        .tempfile()
        .chain_err(|| "creating partition stash file")?;
    let path = stash.path().to_owned();
    eprintln!("Storing saved partition entries to {}", path.display());
    stash
        .as_file()
        .set_len(1024 * 1024)
        .chain_err(|| format!("extending partition stash file {}", path.display()))?;
    saved
        .write(&path)
        .chain_err(|| format!("stashing saved partitions to {}", path.display()))?;
    stash
        .keep()
        .chain_err(|| format!("retaining saved partition stash in {}", path.display()))?;
    Ok(())
}

fn is_dasd(config: &InstallConfig) -> Result<bool> {
    let target = canonicalize(&config.device)
        .chain_err(|| format!("getting absolute path to {}", config.device))?;
    Ok(target.to_string_lossy().starts_with("/dev/dasd"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_id() {
        let orig_content = "options ignition.platform.id=metal foo bar";
        let new_content = bls_entry_write_platform(orig_content, "openstack").unwrap();
        assert_eq!(
            new_content,
            "options ignition.platform.id=openstack foo bar"
        );

        let orig_content = "options foo ignition.platform.id=metal bar";
        let new_content = bls_entry_write_platform(orig_content, "openstack").unwrap();
        assert_eq!(
            new_content,
            "options foo ignition.platform.id=openstack bar"
        );

        let orig_content = "options foo bar ignition.platform.id=metal";
        let new_content = bls_entry_write_platform(orig_content, "openstack").unwrap();
        assert_eq!(
            new_content,
            "options foo bar ignition.platform.id=openstack"
        );
    }

    #[test]
    fn test_options_edit() {
        let orig_content = "options foo bar foobar";

        let delete_kargs = vec!["foo".into()];
        let new_content =
            bls_entry_delete_and_append_kargs(orig_content, Some(&delete_kargs), None).unwrap();
        assert_eq!(new_content, "options bar foobar\n");

        let delete_kargs = vec!["bar".into()];
        let new_content =
            bls_entry_delete_and_append_kargs(orig_content, Some(&delete_kargs), None).unwrap();
        assert_eq!(new_content, "options foo foobar\n");

        let delete_kargs = vec!["foobar".into()];
        let new_content =
            bls_entry_delete_and_append_kargs(orig_content, Some(&delete_kargs), None).unwrap();
        assert_eq!(new_content, "options foo bar\n");

        let delete_kargs = vec!["bar".into(), "foo".into()];
        let new_content =
            bls_entry_delete_and_append_kargs(orig_content, Some(&delete_kargs), None).unwrap();
        assert_eq!(new_content, "options foobar\n");

        let orig_content = "options foo=val bar baz=val";

        let delete_kargs = vec!["foo=val".into()];
        let new_content =
            bls_entry_delete_and_append_kargs(orig_content, Some(&delete_kargs), None).unwrap();
        assert_eq!(new_content, "options bar baz=val\n");

        let delete_kargs = vec!["baz=val".into()];
        let new_content =
            bls_entry_delete_and_append_kargs(orig_content, Some(&delete_kargs), None).unwrap();
        assert_eq!(new_content, "options foo=val bar\n");

        let orig_content =
            "options foo mitigations=auto,nosmt console=tty0 bar console=ttyS0,115200n8 baz";

        let delete_kargs = vec![
            "mitigations=auto,nosmt".into(),
            "console=ttyS0,115200n8".into(),
        ];
        let append_kargs = vec!["console=ttyS1,115200n8".into()];
        let new_content = bls_entry_delete_and_append_kargs(
            orig_content,
            Some(&delete_kargs),
            Some(&append_kargs),
        )
        .unwrap();
        assert_eq!(
            new_content,
            "options foo console=tty0 bar baz console=ttyS1,115200n8\n"
        );
    }
}
