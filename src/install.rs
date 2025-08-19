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
use nix::mount;
use regex::{Captures, Regex};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, BufReader, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::blockdev::*;
use crate::cmdline::*;
use crate::download::*;
use crate::io::*;
#[cfg(target_arch = "s390x")]
use crate::s390x;
use crate::source::*;

// Match the grub.cfg console settings commands in
// https://github.com/coreos/coreos-assembler/blob/main/src/grub.cfg
const GRUB_CFG_CONSOLE_SETTINGS_RE: &str = r"(?P<prefix>\n# CONSOLE-SETTINGS-START\n)(?P<commands>([^\n]*\n)*)(?P<suffix># CONSOLE-SETTINGS-END\n)";

pub fn install(config: InstallConfig) -> Result<()> {
    // evaluate config files
    let config = config.expand_config_files()?;

    // make sure we have a device path
    let device = config
        .dest_device
        .as_deref()
        .context("destination device must be specified")?;

    // find Ignition config and do some simple validation
    let mut ignition = if let Some(file) = &config.ignition_file {
        Some(
            OpenOptions::new()
                .read(true)
                .open(file)
                .with_context(|| format!("opening source Ignition config {file}"))?,
        )
    } else if let Some(url) = &config.ignition_url {
        if url.scheme() == "http" {
            if config.ignition_hash.is_none() && !config.insecure_ignition {
                bail!("refusing to fetch Ignition config over HTTP without --ignition-hash or --insecure-ignition");
            }
        } else if url.scheme() != "https" {
            bail!("unknown protocol for URL '{}'", url);
        }
        Some(
            download_to_tempfile(url, config.fetch_retries)
                .with_context(|| format!("downloading source Ignition config {url}"))?,
        )
    } else {
        None
    };
    if let Some(mut file) = ignition.as_mut() {
        // make sure we have valid JSON and not e.g. an HTML page.
        // we don't parse with the ignition-config crate because its parser
        // rejects unrecognized config versions, and we want to allow those.
        // iso/pxe customize are more restrictive because they want to
        // manipulate the config, but for us it's an opaque blob.
        let reader = BufReader::with_capacity(BUFFER_SIZE, &mut file);
        serde_json::from_reader::<_, serde_json::Value>(reader)
            .context("parsing specified Ignition config")?;
        file.rewind().context("rewinding Ignition config file")?;
    }

    // find network config
    // If the user requested us to copy networking config by passing
    // -n or --copy-network then copy networking config from the
    // directory defined by --network-dir.
    let network_config = if config.copy_network {
        Some(config.network_dir.as_str())
    } else {
        None
    };

    // parse partition saving filters
    let save_partitions = parse_partition_filters(
        &config
            .save_partlabel
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
        &config
            .save_partindex
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
    )?;

    // compute sector size
    // Uninitialized ECKD DASD's blocksize is 512, but after formatting
    // it changes to the recommended 4096
    // https://bugzilla.redhat.com/show_bug.cgi?id=1905159
    #[allow(clippy::match_bool, clippy::match_single_binding)]
    let sector_size = match is_dasd(device, None)
        .with_context(|| format!("checking whether {device} is an IBM DASD disk"))?
    {
        #[cfg(target_arch = "s390x")]
        true => s390x::dasd_try_get_sector_size(device).transpose(),
        _ => None,
    };
    let sector_size = sector_size
        .unwrap_or_else(|| get_sector_size_for_path(Path::new(device)))
        .with_context(|| format!("getting sector size of {device}"))?
        .get();

    // Set up DASD.  We need to do this before initiating the download
    // because otherwise the download might time out while we're low-level
    // formatting the DASD.
    #[cfg(target_arch = "s390x")]
    {
        if is_dasd(device, None)? {
            if !save_partitions.is_empty() {
                // The user requested partition saving, but SavedPartitions
                // doesn't understand DASD VTOCs and won't find any partitions
                // to save.
                bail!("saving DASD partitions is not supported");
            }
            s390x::prepare_dasd(device)?;
        }
    }

    // set up image source
    // create location
    let location: Box<dyn ImageLocation> = if let Some(image_file) = &config.image_file {
        Box::new(FileLocation::new(image_file))
    } else if let Some(image_url) = &config.image_url {
        Box::new(UrlLocation::new(image_url, config.fetch_retries))
    } else if config.offline {
        match OsmetLocation::new(config.architecture.as_str(), sector_size)? {
            Some(osmet) => Box::new(osmet),
            None => bail!("cannot perform offline install; metadata missing"),
        }
    } else {
        // For now, using --stream automatically will cause a download. In the future, we could
        // opportunistically use osmet if the version and stream match an osmet file/the live ISO.

        let maybe_osmet = match config.stream {
            Some(_) => None,
            None => OsmetLocation::new(config.architecture.as_str(), sector_size)?,
        };

        if let Some(osmet) = maybe_osmet {
            Box::new(osmet)
        } else {
            let format = match sector_size {
                4096 => "4k.raw.xz",
                512 => "raw.xz",
                n => {
                    // could bail on non-512, but let's be optimistic and just warn but try the regular
                    // 512b image
                    eprintln!(
                        "Found non-standard sector size {n} for {device}, assuming 512b-compatible"
                    );
                    "raw.xz"
                }
            };
            Box::new(StreamLocation::new(
                config.stream.as_deref().unwrap_or("stable"),
                config.architecture.as_str(),
                "metal",
                format,
                config.stream_base_url.as_ref(),
                config.fetch_retries,
            )?)
        }
    };
    // report it to the user
    eprintln!("{location}");
    // we only support installing from a single artifact
    let mut sources = location.sources()?;
    let mut source = sources.pop().context("no artifacts found")?;
    if !sources.is_empty() {
        bail!("found multiple artifacts");
    }
    if source.signature.is_none() && location.require_signature() {
        if config.insecure {
            eprintln!("Signature not found; skipping verification as requested");
        } else {
            bail!("--insecure not specified and signature not found");
        }
    }

    // open output; ensure it's a block device and we have exclusive access
    let mut dest = OpenOptions::new()
        .read(true)
        .write(true)
        .open(device)
        .with_context(|| format!("opening {device}"))?;
    if !dest
        .metadata()
        .with_context(|| format!("getting metadata for {device}"))?
        .file_type()
        .is_block_device()
    {
        bail!("{} is not a block device", device);
    }
    ensure_exclusive_access(device)
        .with_context(|| format!("checking for exclusive access to {device}"))?;

    // save partitions that we plan to keep
    let saved = SavedPartitions::new_from_disk(&mut dest, &save_partitions)
        .with_context(|| format!("saving partitions from {device}"))?;

    // get reference to partition table
    // For kpartx partitioning, this will conditionally call kpartx -d
    // when dropped
    let mut table = Disk::new(device)?
        .get_partition_table()
        .with_context(|| format!("getting partition table for {device}"))?;

    // copy and postprocess disk image
    // On failure, clear and reread the partition table to prevent the disk
    // from accidentally being used.
    dest.rewind().with_context(|| format!("seeking {device}"))?;
    if let Err(err) = write_disk(
        &config,
        &mut source,
        &mut dest,
        &mut *table,
        &saved,
        ignition,
        network_config,
    ) {
        // log the error so the details aren't dropped if we encounter
        // another error during cleanup
        eprintln!("\nError: {err:?}\n");

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
            reset_partition_table(&config, &mut dest, &mut *table, &saved)?;
        }

        // return a generic error so our exit status is right
        bail!("install failed");
    }

    // Because grub picks /boot by label and the OS picks /boot, we can end up racing/flapping
    // between picking a /boot partition on startup. So check amount of filesystems labeled 'boot'
    // and warn user if it's not only one
    match get_filesystems_with_label("boot", true) {
        Ok(pts) => {
            if pts.len() > 1 {
                let rootdev = fs::canonicalize(device)
                    .unwrap_or_else(|_| PathBuf::from(device))
                    .to_string_lossy()
                    .to_string();
                let pts = pts
                    .iter()
                    .filter(|pt| !pt.contains(&rootdev))
                    .collect::<Vec<_>>();
                eprintln!("\nNote: detected other devices with a filesystem labeled `boot`:");
                for pt in pts {
                    eprintln!("  - {pt}");
                }
                eprintln!("The installed OS may not work correctly if there are multiple boot filesystems.
Before rebooting, investigate whether these filesystems are needed and consider
wiping them with `wipefs -a`.\n"
                );
            }
        }
        Err(e) => eprintln!("checking filesystems labeled 'boot': {e:?}"),
    }

    eprintln!("Install complete.");
    Ok(())
}

fn parse_partition_filters(labels: &[&str], indexes: &[&str]) -> Result<Vec<PartitionFilter>> {
    use PartitionFilter::*;
    let mut filters: Vec<PartitionFilter> = Vec::new();

    // partition label globs
    for glob in labels {
        let filter = Label(
            glob::Pattern::new(glob)
                .with_context(|| format!("couldn't parse label glob '{glob}'"))?,
        );
        filters.push(filter);
    }

    // partition index ranges
    let parse_index = |i: &str| -> Result<Option<NonZeroU32>> {
        match i {
            "" => Ok(None), // open end of range
            _ => Ok(Some(
                NonZeroU32::new(
                    i.parse()
                        .with_context(|| format!("couldn't parse partition index '{i}'"))?,
                )
                .context("partition index cannot be zero")?,
            )),
        }
    };
    for range in indexes {
        let parts: Vec<&str> = range.split('-').collect();
        let filter = match parts.len() {
            1 => Index(parse_index(parts[0])?, parse_index(parts[0])?),
            2 => Index(parse_index(parts[0])?, parse_index(parts[1])?),
            _ => bail!("couldn't parse partition index range '{}'", range),
        };
        match filter {
            Index(None, None) => bail!(
                "both ends of partition index range '{}' cannot be open",
                range
            ),
            Index(Some(x), Some(y)) if x > y => bail!(
                "start of partition index range '{}' cannot be greater than end",
                range
            ),
            _ => filters.push(filter),
        };
    }
    Ok(filters)
}

fn ensure_exclusive_access(device: &str) -> Result<()> {
    let mut parts = Disk::new(device)?.get_busy_partitions()?;
    if parts.is_empty() {
        return Ok(());
    }
    parts.sort_unstable_by_key(|p| p.path.to_string());
    eprintln!("Partitions in use on {device}:");
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
    ignition: Option<File>,
    network_config: Option<&str>,
) -> Result<()> {
    let device = config.dest_device.as_deref().expect("device missing");

    // Get sector size of destination, for comparing with image
    let sector_size = get_sector_size(dest)?;

    // copy the image
    #[allow(clippy::match_bool, clippy::match_single_binding)]
    let image_copy = match is_dasd(device, Some(dest))? {
        #[cfg(target_arch = "s390x")]
        true => s390x::image_copy_s390x,
        _ => image_copy_default,
    };
    write_image(
        source,
        dest,
        Path::new(device),
        image_copy,
        true,
        Some(saved),
        Some(sector_size),
        VerifyKeys::Production,
    )?;
    table.reread()?;

    // postprocess
    if ignition.is_some()
        || config.firstboot_args.is_some()
        || !config.append_karg.is_empty()
        || !config.delete_karg.is_empty()
        || config.platform.is_some()
        || !config.console.is_empty()
        || network_config.is_some()
        || cfg!(target_arch = "s390x")
    {
        let mount = Disk::new(device)?.mount_partition_by_label("boot", mount::MsFlags::empty())?;
        if let Some(ignition) = ignition.as_ref() {
            write_ignition(mount.mountpoint(), &config.ignition_hash, ignition)
                .context("writing Ignition configuration")?;
        }
        if let Some(platform) = config.platform.as_ref() {
            write_platform(mount.mountpoint(), platform).context("writing platform ID")?;
        }
        if config.platform.is_some() || !config.console.is_empty() {
            write_console(
                mount.mountpoint(),
                config.platform.as_deref(),
                &config.console,
            )
            .context("configuring console")?;
        }
        if let Some(firstboot_args) = config.firstboot_args.as_ref() {
            write_firstboot_kargs(mount.mountpoint(), firstboot_args)
                .context("writing firstboot kargs")?;
        }
        if !config.append_karg.is_empty() || !config.delete_karg.is_empty() {
            eprintln!("Modifying kernel arguments");

            Console::maybe_warn_on_kargs(&config.append_karg, "--append-karg", "--console");
            visit_bls_entry_options(mount.mountpoint(), |orig_options: &str| {
                KargsEditor::new()
                    .append(config.append_karg.as_slice())
                    .delete(config.delete_karg.as_slice())
                    .maybe_apply_to(orig_options)
            })
            .context("deleting and appending kargs")?;
        }
        if let Some(network_config) = network_config.as_ref() {
            copy_network_config(mount.mountpoint(), network_config)?;
        }
        #[cfg(target_arch = "s390x")]
        {
            s390x::zipl(
                mount.mountpoint(),
                None,
                None,
                s390x::ZiplSecexMode::Disable,
                None,
            )?;
            s390x::chreipl(device)?;
            if config.secure_ipl {
                s390x::set_loaddev(device)?;
            }
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
    if let Some(digest) = &digest_in {
        digest
            .validate(&mut config_in)
            .context("failed to validate Ignition configuration digest")?;
        config_in
            .rewind()
            .context("rewinding Ignition configuration file")?;
    };

    // make parent directory
    let mut config_dest = mountpoint.to_path_buf();
    config_dest.push("ignition");
    if !config_dest.is_dir() {
        fs::create_dir_all(&config_dest).with_context(|| {
            format!(
                "creating Ignition config directory {}",
                config_dest.display()
            )
        })?;
        // Ignition data may contain secrets; restrict to root
        fs::set_permissions(&config_dest, Permissions::from_mode(0o700)).with_context(|| {
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
    fs::set_permissions(&config_dest, Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "setting file mode for destination Ignition config {}",
            config_dest.display()
        )
    })?;
    io::copy(&mut config_in, &mut config_out).context("writing Ignition config")?;

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
    let contents = format!("set ignition_network_kcmdline=\"{args}\"\n");
    config_out
        .write_all(contents.as_bytes())
        .context("writing first-boot kernel arguments")?;

    Ok(())
}

#[derive(Clone, Default, Deserialize)]
struct PlatformSpec {
    #[serde(default)]
    grub_commands: Vec<String>,
    #[serde(default)]
    kernel_arguments: Vec<String>,
}

/// Override the platform ID.
fn write_platform(mountpoint: &Path, platform: &str) -> Result<()> {
    // early return if setting the platform to the default value, since
    // otherwise we'll think we failed to set it
    if platform == "metal" {
        return Ok(());
    }
    eprintln!("Setting platform to {platform}");

    // We assume that we will only install from metal images and that the
    // bootloader configs will always set ignition.platform.id.
    visit_bls_entry_options(mountpoint, |orig_options: &str| {
        let new_options = KargsEditor::new()
            .replace(&[format!("ignition.platform.id=metal={platform}")])
            .apply_to(orig_options)
            .context("setting platform ID argument")?;
        if orig_options == new_options {
            bail!("couldn't locate platform ID");
        }
        Ok(Some(new_options))
    })?;
    Ok(())
}

/// Configure console kernel arguments and GRUB commands.
fn write_console(mountpoint: &Path, platform: Option<&str>, consoles: &[Console]) -> Result<()> {
    // read platforms table
    let platforms = match fs::read_to_string(mountpoint.join("coreos/platforms.json")) {
        Ok(json) => serde_json::from_str::<HashMap<String, PlatformSpec>>(&json)
            .context("parsing platform table")?,
        // no table for this image?
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(e).context("reading platform table"),
    };

    let mut kargs = Vec::new();
    let mut grub_commands = Vec::new();
    if !consoles.is_empty() {
        // custom console settings completely override platform-specific
        // defaults
        let mut grub_terminals = Vec::new();
        for console in consoles {
            kargs.push(console.karg());
            if let Some(cmd) = console.grub_command() {
                grub_commands.push(cmd);
            }
            grub_terminals.push(console.grub_terminal());
        }
        grub_terminals.sort_unstable();
        grub_terminals.dedup();
        for direction in ["input", "output"] {
            grub_commands.push(format!("terminal_{direction} {}", grub_terminals.join(" ")));
        }
    } else if let Some(platform) = platform {
        // platform-specific defaults
        if platform == "metal" {
            // we're just being asked to apply the defaults which are already
            // applied
            return Ok(());
        }
        let spec = platforms.get(platform).cloned().unwrap_or_default();
        kargs.extend(spec.kernel_arguments);
        grub_commands.extend(spec.grub_commands);
    } else {
        // nothing to do and the caller shouldn't have called us
        unreachable!();
    }

    // set kargs, removing any metal-specific ones
    let metal_spec = platforms.get("metal").cloned().unwrap_or_default();
    visit_bls_entry_options(mountpoint, |orig_options: &str| {
        KargsEditor::new()
            .append(&kargs)
            .delete(&metal_spec.kernel_arguments)
            .maybe_apply_to(orig_options)
            .context("setting platform kernel arguments")
    })?;

    // set grub commands
    if grub_commands != metal_spec.grub_commands {
        // prefer the new grub2/console.cfg, but fallback to grub2/grub.cfg
        let mut name = "grub2/console.cfg";
        let mut path = mountpoint.join(name);
        if !path.exists() {
            name = "grub2/grub.cfg";
            path = mountpoint.join(name);
        }
        let grub_cfg = fs::read_to_string(&path).with_context(|| format!("reading {name}"))?;
        let new_grub_cfg = update_grub_cfg_console_settings(&grub_cfg, &grub_commands)
            .with_context(|| format!("updating {name}"))?;
        fs::write(&path, new_grub_cfg).with_context(|| format!("writing {name}"))?;
    }
    Ok(())
}

/// Rewrite the grub.cfg CONSOLE-SETTINGS block to use the specified GRUB
/// commands, and return the result.
fn update_grub_cfg_console_settings(grub_cfg: &str, commands: &[String]) -> Result<String> {
    let mut new_commands = commands.join("\n");
    if !new_commands.is_empty() {
        new_commands.push('\n');
    }
    let re = Regex::new(GRUB_CFG_CONSOLE_SETTINGS_RE).unwrap();
    if !re.is_match(grub_cfg) {
        bail!("missing substitution marker in grub.cfg");
    }
    Ok(re
        .replace(grub_cfg, |caps: &Captures| {
            format!(
                "{}{}{}",
                caps.name("prefix").expect("didn't match prefix").as_str(),
                new_commands,
                caps.name("suffix").expect("didn't match suffix").as_str()
            )
        })
        .into_owned())
}

/// Copy networking config if asked to do so
fn copy_network_config(mountpoint: &Path, net_config_src: &str) -> Result<()> {
    eprintln!("Copying networking configuration from {net_config_src}");

    // get the path to the destination directory
    let net_config_dest = mountpoint.join("coreos-firstboot-network");

    // make the directory if it doesn't exist
    fs::create_dir_all(&net_config_dest).with_context(|| {
        format!(
            "creating destination networking config directory {}",
            net_config_dest.display()
        )
    })?;

    // copy files from source to destination directories
    for entry in fs::read_dir(net_config_src)
        .with_context(|| format!("reading directory {net_config_src}"))?
    {
        let entry = entry.with_context(|| format!("reading directory {net_config_src}"))?;
        let srcpath = entry.path();
        let destpath = net_config_dest.join(entry.file_name());
        if srcpath.is_file() {
            eprintln!("Copying {} to installed system", srcpath.display());
            fs::copy(&srcpath, destpath).context("Copying networking config")?;
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
    let device = config.dest_device.as_deref().expect("device missing");

    if is_dasd(device, Some(dest))? {
        // Don't write out a GPT, since the backup GPT may overwrite
        // something we're not allowed to touch.  Just clear the first MiB
        // of disk.
        dest.rewind().context("seeking to start of disk")?;
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
    fn test_parse_partition_filters() {
        use PartitionFilter::*;

        let g = |v| Label(glob::Pattern::new(v).unwrap());
        let i = |v| Some(NonZeroU32::new(v).unwrap());

        assert_eq!(
            parse_partition_filters(&["foo", "z*b?", ""], &["1", "7-7", "2-4", "-3", "4-"])
                .unwrap(),
            vec![
                g("foo"),
                g("z*b?"),
                g(""),
                Index(i(1), i(1)),
                Index(i(7), i(7)),
                Index(i(2), i(4)),
                Index(None, i(3)),
                Index(i(4), None)
            ]
        );

        let bad_globs = vec![("***", "couldn't parse label glob '***'")];
        for (glob, err) in bad_globs {
            assert_eq!(
                &parse_partition_filters(&["f", glob, "z*"], &["7-", "34"])
                    .unwrap_err()
                    .to_string(),
                err
            );
        }

        let bad_ranges = vec![
            ("", "both ends of partition index range '' cannot be open"),
            ("-", "both ends of partition index range '-' cannot be open"),
            ("--", "couldn't parse partition index range '--'"),
            ("0", "partition index cannot be zero"),
            ("-2-3", "couldn't parse partition index range '-2-3'"),
            ("23q", "couldn't parse partition index '23q'"),
            ("23-45.7", "couldn't parse partition index '45.7'"),
            ("0x7", "couldn't parse partition index '0x7'"),
            (
                "9-7",
                "start of partition index range '9-7' cannot be greater than end",
            ),
        ];
        for (range, err) in bad_ranges {
            assert_eq!(
                &parse_partition_filters(&["f", "z*"], &["7-", range, "34"])
                    .unwrap_err()
                    .to_string(),
                err
            );
        }
    }

    #[test]
    fn test_update_grub_cfg() {
        let base_cfgs = vec![
            // no existing commands
            "a\nb\n# CONSOLE-SETTINGS-START\n# CONSOLE-SETTINGS-END\nc\nd",
            // one existing command
            "a\nb\n# CONSOLE-SETTINGS-START\nas df\n# CONSOLE-SETTINGS-END\nc\nd",
            // multiple existing commands
            "a\nb\n# CONSOLE-SETTINGS-START\nas df\nghjkl\n# CONSOLE-SETTINGS-END\nc\nd",
        ];
        for cfg in base_cfgs {
            // no new commands
            assert_eq!(
                update_grub_cfg_console_settings(cfg, &[]).unwrap(),
                "a\nb\n# CONSOLE-SETTINGS-START\n# CONSOLE-SETTINGS-END\nc\nd"
            );
            // one new command
            assert_eq!(
                update_grub_cfg_console_settings(cfg, &["first".into()]).unwrap(),
                "a\nb\n# CONSOLE-SETTINGS-START\nfirst\n# CONSOLE-SETTINGS-END\nc\nd"
            );
            // multiple new commands
            assert_eq!(
                update_grub_cfg_console_settings(cfg, &["first".into(), "sec ond".into(), "third".into()]).unwrap(),
                "a\nb\n# CONSOLE-SETTINGS-START\nfirst\nsec ond\nthird\n# CONSOLE-SETTINGS-END\nc\nd"
            );
        }

        // missing substitution marker
        update_grub_cfg_console_settings("a\nb\nc\nd", &[]).unwrap_err();
    }
}
