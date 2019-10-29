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

mod blockdev;
mod errors;
mod source;
mod verify;

use byte_unit::Byte;
use clap::{crate_version, App, AppSettings, Arg, SubCommand};
use error_chain::{bail, quick_main, ChainedError};
use flate2::read::GzDecoder;
use progress_streams::ProgressReader;
use reqwest::Url;
use std::fs::{create_dir_all, read_dir, File, OpenOptions};
use std::io::{copy, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::time::{Duration, Instant};
use xz2::read::XzDecoder;

use crate::blockdev::*;
use crate::errors::*;
use crate::source::*;
use crate::verify::*;

struct Config {
    device: String,
    location: Box<dyn ImageLocation>,
    ignition: Option<String>,
    platform: Option<String>,
    firstboot_kargs: Option<String>,
    insecure: bool,
}

quick_main!(run);

fn run() -> Result<()> {
    // get arguments
    let config = parse_args().chain_err(|| "parsing arguments")?;

    // set up image source
    let mut source = config.location.source()?;
    if source.signature.is_none() {
        if config.insecure {
            eprintln!("Signature not found; skipping verification as requested");
        } else {
            bail!("--insecure not specified and signature not found");
        }
    }

    // open output; ensure it's a block device and we have exclusive access
    let mut dest = OpenOptions::new()
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
    reread_partition_table(&dest)
        .chain_err(|| format!("checking for exclusive access to {}", &config.device))?;

    // copy and postprocess disk image
    // On failure, clear and reread the partition table to prevent the disk
    // from accidentally being used.
    if let Err(err) = write_disk(&config, &mut source, &mut dest) {
        // log the error so the details aren't dropped if we encounter
        // another error during cleanup
        eprint!("{}", ChainedError::display_chain(&err));

        // clean up
        clear_partition_table(&mut dest)?;

        // return a generic error so our exit status is right
        bail!("install failed");
    }

    println!("Install complete.");
    Ok(())
}

/// Parse command-line arguments.
fn parse_args() -> Result<Config> {
    let uname = nix::sys::utsname::uname();
    // Args are listed in --help in the order declared here.  Please keep
    // the entire help text to 80 columns.
    let app_matches = App::new("coreos-installer")
        .version(crate_version!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .global_setting(AppSettings::ArgsNegateSubcommands)
        .global_setting(AppSettings::DeriveDisplayOrder)
        .global_setting(AppSettings::DisableHelpSubcommand)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .global_setting(AppSettings::VersionlessSubcommands)
        .subcommand(
            SubCommand::with_name("install")
                .about("Install Fedora CoreOS or RHEL CoreOS")
                // ways to specify the image source
                .arg(
                    Arg::with_name("stream")
                        .short("s")
                        .long("stream")
                        .value_name("name")
                        .help("Fedora CoreOS stream")
                        .default_value("stable")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("image-url")
                        .short("u")
                        .long("image-url")
                        .value_name("URL")
                        .help("Manually specify the image URL")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("image-file")
                        .short("f")
                        .long("image-file")
                        .value_name("path")
                        .help("Manually specify a local image file")
                        .takes_value(true),
                )
                // postprocessing options
                .arg(
                    Arg::with_name("ignition-path")
                        .short("i")
                        .long("ignition")
                        .value_name("path")
                        .help("Embed an Ignition config")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("platform")
                        .short("p")
                        .long("platform")
                        .value_name("name")
                        .help("Override the Ignition platform ID")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("firstboot-kargs")
                        .long("firstboot-args")
                        .value_name("args")
                        .help("Additional kernel args for the first boot")
                        .takes_value(true),
                )
                // obscure options without short names
                .arg(
                    Arg::with_name("insecure")
                        .long("insecure")
                        .help("Skip signature verification"),
                )
                .arg(
                    Arg::with_name("stream-base-url")
                        .long("stream-base-url")
                        .value_name("URL")
                        .help("Base URL for Fedora CoreOS stream metadata")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("architecture")
                        .long("architecture")
                        .value_name("name")
                        .help("Target CPU architecture")
                        .default_value(uname.machine())
                        .takes_value(true),
                )
                // positional args
                .arg(
                    Arg::with_name("device")
                        .help("Destination device")
                        .required(true)
                        .takes_value(true),
                ),
        )
        .get_matches();
    let matches = app_matches
        .subcommand_matches("install")
        .expect("install subcommand arguments missing");

    // Build image location.  Ideally we'd use conflicts_with (and an
    // ArgGroup for streams), but that doesn't play well with default
    // arguments, so we manually prioritize modes.
    let location: Box<dyn ImageLocation> = if matches.is_present("image-file") {
        Box::new(FileLocation::new(
            matches.value_of("image-file").expect("image-file missing"),
        ))
    } else if matches.is_present("image-url") {
        let image_url = Url::parse(matches.value_of("image-url").expect("image-url missing"))
            .chain_err(|| "parsing image URL")?;
        Box::new(UrlLocation::new(&image_url))
    } else {
        let base_url = if matches.is_present("stream-base-url") {
            Some(
                Url::parse(
                    matches
                        .value_of("stream-base-url")
                        .expect("stream-base-url missing"),
                )
                .chain_err(|| "parsing stream base URL")?,
            )
        } else {
            None
        };
        Box::new(StreamLocation::new(
            matches.value_of("stream").expect("stream missing"),
            matches
                .value_of("architecture")
                .expect("architecture missing"),
            base_url.as_ref(),
        )?)
    };
    // and report it to the user
    println!("{}", location);

    // build configuration
    Ok(Config {
        device: matches
            .value_of("device")
            .map(String::from)
            .expect("device missing"),
        location,
        ignition: matches.value_of("ignition-path").map(String::from),
        platform: matches.value_of("platform").map(String::from),
        firstboot_kargs: matches.value_of("firstboot-kargs").map(String::from),
        insecure: matches.is_present("insecure"),
    })
}

/// Copy the image source to the target disk and do all post-processing.
/// If this function fails, the caller should wipe the partition table
/// to ensure the user doesn't boot from a partially-written disk.
fn write_disk(config: &Config, source: &mut ImageSource, dest: &mut File) -> Result<()> {
    // copy the image
    write_image(source, dest)?;
    reread_partition_table(dest)?;
    udev_settle()?;

    // postprocess
    if config.ignition.is_some() || config.firstboot_kargs.is_some() || config.platform.is_some() {
        let mount = mount_boot(&config.device)?;
        if let Some(ignition) = config.ignition.as_ref() {
            write_ignition(mount.mountpoint(), ignition)?;
        }
        if let Some(firstboot_kargs) = config.firstboot_kargs.as_ref() {
            write_firstboot_kargs(mount.mountpoint(), firstboot_kargs)?;
        }
        if let Some(platform) = config.platform.as_ref() {
            write_platform(mount.mountpoint(), platform)?;
        }
    }

    Ok(())
}

/// Copy the image to disk and verify its signature.
fn write_image(source: &mut ImageSource, dest: &mut File) -> Result<()> {
    // wrap source for GPG verification
    let mut verify_reader: Box<dyn Read> = {
        if let Some(signature) = source.signature.as_ref() {
            Box::new(GpgReader::new(&mut source.reader, signature)?)
        } else {
            Box::new(&mut source.reader)
        }
    };

    // wrap again for progress reporting
    let have_length = source.length_hint.is_some();
    let length_hint = source.length_hint.unwrap_or(0);
    let mut position: u64 = 0;
    let mut last_report = Instant::now();
    let mut progress_reader = ProgressReader::new(&mut verify_reader, |progress: usize| {
        position += progress as u64;
        if last_report.elapsed() >= Duration::from_secs(1)
            || (have_length && position == length_hint)
        {
            last_report = Instant::now();
            if have_length {
                print!(
                    "> Read {}/{} ({}%)   \r",
                    format_bytes(position),
                    format_bytes(length_hint),
                    100 * position / length_hint
                );
            } else {
                print!("> Read {}   \r", format_bytes(position));
            }
            let _ = std::io::stdout().flush();
        }
    });

    // Wrap in a BufReader so we can peek at the first few bytes for
    // format sniffing.  Don't trust the content-type since the server
    // may not be configured correctly, or the file might be local.
    // Then wrap in a reader for decompression.
    let mut buf_reader = BufReader::new(&mut progress_reader);
    let mut decompress_reader: Box<dyn Read> = {
        let sniff = buf_reader.fill_buf().chain_err(|| "sniffing input")?;
        // verify default buffer size >= the largest magic number we might
        // care about
        assert!(sniff.len() >= 8);
        if &sniff[0..2] == b"\x1f\x8b" {
            Box::new(GzDecoder::new(buf_reader))
        } else if &sniff[0..6] == b"\xfd7zXZ\x00" {
            Box::new(XzDecoder::new(buf_reader))
        } else {
            Box::new(buf_reader)
        }
    };

    // Cache the first MiB of input and write zeroes instead.  This ensures
    // that the disk image can't be used accidentally before its GPG signature
    // is verified.
    let mut first_mb: [u8; 1024 * 1024] = [0; 1024 * 1024];
    dest.write_all(&first_mb)
        .chain_err(|| "clearing first MiB of disk")?;
    decompress_reader
        .read_exact(&mut first_mb)
        .chain_err(|| "decoding first MiB of image")?;

    // do the rest of the copy
    copy(&mut decompress_reader, dest).chain_err(|| "decoding and writing image")?;

    // verify_reader has now checked the signature, so fill in the first MiB
    dest.seek(SeekFrom::Start(0))
        .chain_err(|| "seeking to start of disk")?;
    dest.write_all(&first_mb)
        .chain_err(|| "writing to first MiB of disk")?;

    // flush
    dest.flush().chain_err(|| "flushing data to disk")?;
    dest.sync_all().chain_err(|| "syncing data to disk")?;

    // log final newline
    println!();

    Ok(())
}

/// Write the Ignition config.
fn write_ignition(mountpoint: &Path, config_src: &str) -> Result<()> {
    println!("Writing Ignition config");

    // make parent directory
    let mut config_dest = mountpoint.to_path_buf();
    config_dest.push("ignition");
    create_dir_all(&config_dest).chain_err(|| "creating Ignition config directory")?;

    // do the copy
    config_dest.push("config.ign");
    let mut config_in = OpenOptions::new()
        .read(true)
        .open(config_src)
        .chain_err(|| format!("opening source Ignition config {}", config_src))?;
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
    println!("Writing first-boot kernel arguments");

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

/// Override the platform ID.
fn write_platform(mountpoint: &Path, platform: &str) -> Result<()> {
    // early return if setting the platform to the default value, since
    // otherwise we'll think we failed to set it
    if platform == "metal" {
        return Ok(());
    }

    println!("Setting platform to {}", platform);

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
            let mut orig_contents = String::new();
            config
                .read_to_string(&mut orig_contents)
                .chain_err(|| format!("reading {}", path.display()))?;

            // Rewrite the config.  Assume that we will only install
            // from metal images and that their bootloader configs will
            // always set ignition.platform.id.  Fail if those
            // assumptions change.  This is deliberately simplistic.
            let new_contents = orig_contents.replace(
                "ignition.platform.id=metal",
                &format!("ignition.platform.id={}", platform),
            );
            if orig_contents == new_contents {
                bail!("Couldn't locate platform ID in {}", path.display());
            }

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

/// Clear the partition table.  For use after a failure.
fn clear_partition_table(dest: &mut File) -> Result<()> {
    println!("Clearing partition table");
    dest.seek(SeekFrom::Start(0))
        .chain_err(|| "seeking to start of disk")?;
    let zeroes: [u8; 1024 * 1024] = [0; 1024 * 1024];
    dest.write_all(&zeroes)
        .chain_err(|| "clearing partition table")?;
    dest.flush()
        .chain_err(|| "flushing partition table to disk")?;
    dest.sync_all()
        .chain_err(|| "syncing partition table to disk")?;
    reread_partition_table(&dest)?;
    udev_settle()?;
    Ok(())
}

/// Format a size in bytes.
fn format_bytes(count: u64) -> String {
    Byte::from_bytes(count.into())
        .get_appropriate_unit(true)
        .format(1)
}
