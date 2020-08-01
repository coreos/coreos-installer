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

use clap::{crate_version, App, AppSettings, Arg, ArgMatches, SubCommand};
use error_chain::bail;
use reqwest::Url;
use std::fs::{File, OpenOptions};
use std::num::NonZeroU32;
use std::path::Path;

use crate::blockdev::*;
use crate::download::*;
use crate::errors::*;
use crate::io::IgnitionHash;
use crate::source::*;

pub enum Config {
    Install(InstallConfig),
    Download(DownloadConfig),
    ListStream(ListStreamConfig),
    IsoEmbed(IsoEmbedConfig),
    IsoShow(IsoShowConfig),
    IsoRemove(IsoRemoveConfig),
    OsmetFiemap(OsmetFiemapConfig),
    OsmetPack(OsmetPackConfig),
    OsmetUnpack(OsmetUnpackConfig),
}

pub struct InstallConfig {
    pub device: String,
    pub location: Box<dyn ImageLocation>,
    pub ignition: Option<File>,
    pub ignition_hash: Option<IgnitionHash>,
    pub platform: Option<String>,
    pub firstboot_kargs: Option<String>,
    pub append_kargs: Option<Vec<String>>,
    pub delete_kargs: Option<Vec<String>>,
    pub insecure: bool,
    pub preserve_on_error: bool,
    pub network_config: Option<String>,
    pub save_partitions: Vec<PartitionFilter>,
}

#[derive(Debug, PartialEq)]
pub enum PartitionFilter {
    Label(glob::Pattern),
    Index(Option<NonZeroU32>, Option<NonZeroU32>),
}

pub struct DownloadConfig {
    pub location: Box<dyn ImageLocation>,
    pub directory: String,
    pub decompress: bool,
    pub insecure: bool,
}

pub struct ListStreamConfig {
    pub stream_base_url: Option<Url>,
    pub stream: String,
}

pub struct IsoEmbedConfig {
    pub input: String,
    pub output: Option<String>,
    pub ignition: Option<String>,
    pub force: bool,
}

pub struct IsoShowConfig {
    pub input: String,
}

pub struct IsoRemoveConfig {
    pub input: String,
    pub output: Option<String>,
}

pub struct OsmetFiemapConfig {
    pub file: String,
}

pub struct OsmetRootBlkDevReal {
    pub underlying_device: String,
    pub offset_sectors: u32,
}

pub struct OsmetPackConfig {
    pub output: String,
    pub device: String,
    pub checksum: String,
    pub description: String,
    pub rootdev: Option<OsmetRootBlkDevReal>,
    pub fast: bool,
}

pub struct OsmetUnpackConfig {
    pub repo: String,
    pub osmet: String,
    pub device: String,
}

/// Parse command-line arguments.
pub fn parse_args() -> Result<Config> {
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
                        .conflicts_with("image-file")
                        .conflicts_with("image-url")
                        .help("Fedora CoreOS stream")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("image-url")
                        .short("u")
                        .long("image-url")
                        .conflicts_with("stream")
                        .conflicts_with("image-file")
                        .value_name("URL")
                        .help("Manually specify the image URL")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("image-file")
                        .short("f")
                        .long("image-file")
                        .conflicts_with("stream")
                        .conflicts_with("image-url")
                        .value_name("path")
                        .help("Manually specify a local image file")
                        .takes_value(true),
                )
                // postprocessing options
                .arg(
                    Arg::with_name("ignition-file")
                        .short("i")
                        .long("ignition-file")
                        .conflicts_with("ignition-url")
                        // deprecated long name from <= 0.1.2
                        .alias("ignition")
                        .value_name("path")
                        .help("Embed an Ignition config from a file")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("ignition-url")
                        .short("I")
                        .long("ignition-url")
                        .conflicts_with("ignition-file")
                        .value_name("URL")
                        .help("Embed an Ignition config from a URL")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("ignition-hash")
                        .long("ignition-hash")
                        .value_name("digest")
                        .help("Digest (type-value) of the Ignition config")
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
                        .takes_value(true)
                        // This used to be for configuring networking from the cmdline, but it has
                        // been obsoleted by the nicer `--copy-network` approach. We still need it
                        // for now though. It's used at least by `coreos-installer.service`.
                        .hidden(true),
                )
                .arg(
                    Arg::with_name("append-karg")
                        .long("append-karg")
                        .value_name("arg")
                        .help("Append default kernel arg")
                        .takes_value(true)
                        .number_of_values(1)
                        .multiple(true),
                )
                .arg(
                    Arg::with_name("delete-karg")
                        .long("delete-karg")
                        .value_name("arg")
                        .help("Delete default kernel arg")
                        .takes_value(true)
                        .number_of_values(1)
                        .multiple(true),
                )
                .arg(
                    Arg::with_name("copy-network")
                        .short("n")
                        .long("copy-network")
                        .help("Copy network config from install environment"),
                )
                .arg(
                    Arg::with_name("network-dir")
                        .long("network-dir")
                        .value_name("path")
                        .default_value("/etc/NetworkManager/system-connections/")
                        .takes_value(true)
                        .empty_values(false)
                        .help("For use with -n.")
                        .next_line_help(true), // so we can stay under 80 chars
                )
                .arg(
                    Arg::with_name("save-partlabel")
                        .long("save-partlabel")
                        .value_name("lx")
                        .help("Save partitions with this label glob")
                        .takes_value(true)
                        // allow argument multiple times, but one value each
                        .number_of_values(1)
                        .multiple(true)
                        // allow "a,b" in one argument
                        .require_delimiter(true)
                )
                .arg(
                    Arg::with_name("save-partindex")
                        .long("save-partindex")
                        .value_name("id")
                        .help("Save partitions with this number or range")
                        .takes_value(true)
                        // allow argument multiple times, but one value each
                        .number_of_values(1)
                        .multiple(true)
                        // allow "1-5,7" in one argument
                        .require_delimiter(true)
                        // allow ranges like "-2"
                        .allow_hyphen_values(true)
                )
                // obscure options without short names
                .arg(
                    Arg::with_name("offline")
                        .long("offline")
                        .help("Force offline installation"),
                )
                .arg(
                    Arg::with_name("insecure")
                        .long("insecure")
                        .help("Skip signature verification"),
                )
                .arg(
                    Arg::with_name("insecure-ignition")
                        .long("insecure-ignition")
                        .help("Allow Ignition URL without HTTPS or hash"),
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
                .arg(
                    Arg::with_name("preserve-on-error")
                        .long("preserve-on-error")
                        .help("Don't clear partition table on error"),
                )
                // positional args
                .arg(
                    Arg::with_name("device")
                        .help("Destination device")
                        .required(true)
                        .takes_value(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("download")
                .about("Download a CoreOS image")
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
                    Arg::with_name("architecture")
                        .long("architecture")
                        .value_name("name")
                        .help("Target CPU architecture")
                        .default_value(uname.machine())
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("platform")
                        .short("p")
                        .long("platform")
                        .value_name("name")
                        .help("Fedora CoreOS platform name")
                        .default_value("metal")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("format")
                        .short("f")
                        .long("format")
                        .value_name("name")
                        .help("Image format")
                        .default_value("raw.xz")
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
                    Arg::with_name("directory")
                        .short("C")
                        .long("directory")
                        .value_name("path")
                        .help("Destination directory")
                        .default_value(".")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("decompress")
                        .short("d")
                        .long("decompress")
                        .help("Decompress image and don't save signature"),
                )
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
                ),
        )
        .subcommand(
            SubCommand::with_name("list-stream")
                .about("List available images in a Fedora CoreOS stream")
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
                    Arg::with_name("stream-base-url")
                        .long("stream-base-url")
                        .value_name("URL")
                        .help("Base URL for Fedora CoreOS stream metadata")
                        .takes_value(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("iso")
                .about("Embed an Ignition config in a CoreOS live ISO image")
                .subcommand(
                    SubCommand::with_name("embed")
                        .about("Embed an Ignition config in an ISO image")
                        .arg(
                            Arg::with_name("config")
                                .short("c")
                                .long("config")
                                .value_name("path")
                                .help("Ignition config to embed [default: stdin]")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("force")
                                .short("f")
                                .long("force")
                                .help("Overwrite an existing embedded Ignition config"),
                        )
                        .arg(
                            Arg::with_name("output")
                                .short("o")
                                .long("output")
                                .value_name("path")
                                .help("Copy to a new file, instead of modifying in place")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("input")
                                .value_name("ISO")
                                .help("ISO image")
                                .required(true)
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("show")
                        .about("Show the embedded Ignition config from an ISO image")
                        .arg(
                            Arg::with_name("input")
                                .value_name("ISO")
                                .help("ISO image")
                                .required(true)
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("remove")
                        .about("Remove an existing embedded Ignition config from an ISO image")
                        .arg(
                            Arg::with_name("output")
                                .short("o")
                                .long("output")
                                .value_name("path")
                                .help("Copy to a new file, instead of modifying in place")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("input")
                                .value_name("ISO")
                                .help("ISO image")
                                .required(true)
                                .takes_value(true),
                        ),
                ),
        )
        .subcommand(
            SubCommand::with_name("osmet")
                .about("Efficient CoreOS metal disk image packing using OSTree commits")
                // users shouldn't be interacting with this command normally
                .setting(AppSettings::Hidden)
                .subcommand(
                    SubCommand::with_name("pack")
                        .about("Create osmet file from CoreOS block device")
                        .arg(
                            Arg::with_name("output")
                                .long("output")
                                .value_name("FILE")
                                .required(true) // could output to stdout if missing?
                                .help("Path to osmet file to write")
                                .takes_value(true),
                        )
                        .arg(
                            // XXX: rebase on top of
                            // https://github.com/coreos/coreos-installer/pull/178 and use the same
                            // type-digest format
                            Arg::with_name("checksum")
                                .long("checksum")
                                .value_name("SHA256")
                                .required(true)
                                .help("Expected SHA256 of block device")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("description")
                                .long("description")
                                .value_name("TEXT")
                                .required(true)
                                .help("Description of OS")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("real-rootdev")
                                .long("real-rootdev")
                                .value_name("PATH,OFFSET")
                                .help("Underlying device for e.g. RHCOS LUKS container; /dev/disk/by-label/root should be mountable")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("fast")
                                .long("fast")
                                .help("Use worse compression, for development builds")
                        )
                        // positional args
                        .arg(
                            Arg::with_name("device")
                                .help("Source device")
                                .value_name("DEV")
                                .required(true)
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("unpack")
                        .about("Generate raw metal image from osmet file and OSTree repo")
                        .arg(
                            Arg::with_name("osmet")
                                .help("osmet file")
                                .value_name("PATH")
                                .required(true)
                                .long("osmet")
                                .takes_value(true),
                        )
                        // positional args
                        .arg(
                            Arg::with_name("repo")
                                .help("OSTree repo")
                                .value_name("PATH")
                                .required(true)
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("device")
                                .help("Destination device")
                                .value_name("DEV")
                                .required(true)
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("fiemap")
                        .about("Print file extent mapping of specific file")
                        // positional args
                        .arg(
                            Arg::with_name("file")
                                .help("File to map")
                                .value_name("PATH")
                                .required(true)
                                .takes_value(true),
                        ),
                ),
        )
        .get_matches();

    match app_matches.subcommand() {
        ("install", Some(matches)) => parse_install(&matches),
        ("download", Some(matches)) => parse_download(&matches),
        ("list-stream", Some(matches)) => parse_list_stream(&matches),
        ("iso", Some(iso_matches)) => match iso_matches.subcommand() {
            ("embed", Some(matches)) => parse_iso_embed(&matches),
            ("show", Some(matches)) => parse_iso_show(&matches),
            ("remove", Some(matches)) => parse_iso_remove(&matches),
            _ => bail!("unrecognized 'iso' subcommand"),
        },
        ("osmet", Some(osmet_matches)) => match osmet_matches.subcommand() {
            ("pack", Some(matches)) => parse_osmet_pack(&matches),
            ("unpack", Some(matches)) => parse_osmet_unpack(&matches),
            ("fiemap", Some(matches)) => parse_osmet_fiemap(&matches),
            _ => bail!("unrecognized 'osmet' subcommand"),
        },
        _ => bail!("unrecognized subcommand"),
    }
}

fn parse_install(matches: &ArgMatches) -> Result<Config> {
    let device = matches
        .value_of("device")
        .map(String::from)
        .expect("device missing");
    let architecture = matches
        .value_of("architecture")
        .expect("architecture missing");

    let sector_size = get_sector_size_for_path(Path::new(&device))
        .chain_err(|| format!("getting sector size of {}", &device))?
        .get();

    let location: Box<dyn ImageLocation> = if matches.is_present("image-file") {
        Box::new(FileLocation::new(
            matches.value_of("image-file").expect("image-file missing"),
        ))
    } else if matches.is_present("image-url") {
        let image_url = Url::parse(matches.value_of("image-url").expect("image-url missing"))
            .chain_err(|| "parsing image URL")?;
        Box::new(UrlLocation::new(&image_url))
    } else if matches.is_present("offline") {
        match OsmetLocation::new(architecture, sector_size)? {
            Some(osmet) => Box::new(osmet),
            None => bail!("cannot perform offline install; metadata missing"),
        }
    } else {
        // For now, using --stream automatically will cause a download. In the future, we could
        // opportunistically use osmet if the version and stream match an osmet file/the live ISO.

        let maybe_osmet = if matches.is_present("stream") {
            None
        } else {
            OsmetLocation::new(architecture, sector_size)?
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
                        "Found non-standard sector size {} for {}, assuming 512b-compatible",
                        n, &device
                    );
                    "raw.xz"
                }
            };
            let base_url = if let Some(stream_base_url) = matches.value_of("stream-base-url") {
                Some(Url::parse(stream_base_url).chain_err(|| "parsing stream base URL")?)
            } else {
                None
            };
            Box::new(StreamLocation::new(
                matches.value_of("stream").unwrap_or("stable"),
                architecture,
                "metal",
                format,
                base_url.as_ref(),
            )?)
        }
    };

    let ignition = if matches.is_present("ignition-file") {
        matches
            .value_of("ignition-file")
            .map(|file| {
                OpenOptions::new()
                    .read(true)
                    .open(file)
                    .chain_err(|| format!("opening source Ignition config {}", file))
            })
            .transpose()?
    } else if matches.is_present("ignition-url") {
        matches.value_of("ignition-url").map(|url| {
            if url.starts_with("http://") {
                if !matches.is_present("ignition-hash") && !matches.is_present("insecure-ignition") {
                    bail!("refusing to fetch Ignition config over HTTP without --ignition-hash or --insecure-ignition");
                }
            } else if !url.starts_with("https://") {
                bail!("unknown protocol for URL '{}'", url);
            }
            download_to_tempfile(url)
                .chain_err(|| format!("downloading source Ignition config {}", url))
        }).transpose()?
    } else {
        None
    };

    // and report it to the user
    eprintln!("{}", location);

    // If the user requested us to copy networking config by passing
    // -n or --copy-network then copy networking config from the
    // directory defined by --network-dir.
    let network_config = if matches.is_present("copy-network") {
        matches.value_of("network-dir").map(String::from)
    } else {
        None
    };

    // build configuration
    Ok(Config::Install(InstallConfig {
        device,
        location,
        ignition,
        ignition_hash: matches
            .value_of("ignition-hash")
            .map(IgnitionHash::try_parse)
            .transpose()
            .chain_err(|| "parsing Ignition config hash")?,
        platform: matches.value_of("platform").map(String::from),
        firstboot_kargs: matches.value_of("firstboot-kargs").map(String::from),
        append_kargs: matches
            .values_of("append-karg")
            .map(|v| v.map(String::from).collect()),
        delete_kargs: matches
            .values_of("delete-karg")
            .map(|v| v.map(String::from).collect()),
        insecure: matches.is_present("insecure"),
        preserve_on_error: matches.is_present("preserve-on-error"),
        network_config,
        save_partitions: parse_partition_filters(
            &matches
                .values_of("save-partlabel")
                .unwrap_or_default()
                .collect::<Vec<&str>>(),
            &matches
                .values_of("save-partindex")
                .unwrap_or_default()
                .collect::<Vec<&str>>(),
        )?,
    }))
}

fn parse_partition_filters(labels: &[&str], indexes: &[&str]) -> Result<Vec<PartitionFilter>> {
    use PartitionFilter::*;
    let mut filters: Vec<PartitionFilter> = Vec::new();

    // partition label globs
    for glob in labels {
        let filter = Label(
            glob::Pattern::new(glob)
                .chain_err(|| format!("couldn't parse label glob '{}'", glob))?,
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
                        .chain_err(|| format!("couldn't parse partition index '{}'", i))?,
                )
                .chain_err(|| "partition index cannot be zero")?,
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

fn parse_download(matches: &ArgMatches) -> Result<Config> {
    // Build image location.  Ideally we'd use conflicts_with (and an
    // ArgGroup for streams), but that doesn't play well with default
    // arguments, so we manually prioritize modes.
    let location: Box<dyn ImageLocation> = if matches.is_present("image-url") {
        let image_url = Url::parse(matches.value_of("image-url").expect("image-url missing"))
            .chain_err(|| "parsing image URL")?;
        Box::new(UrlLocation::new(&image_url))
    } else {
        let base_url = if let Some(stream_base_url) = matches.value_of("stream-base-url") {
            Some(Url::parse(stream_base_url).chain_err(|| "parsing stream base URL")?)
        } else {
            None
        };
        Box::new(StreamLocation::new(
            matches.value_of("stream").expect("stream missing"),
            matches
                .value_of("architecture")
                .expect("architecture missing"),
            matches.value_of("platform").expect("platform missing"),
            matches.value_of("format").expect("format missing"),
            base_url.as_ref(),
        )?)
    };

    // build configuration
    Ok(Config::Download(DownloadConfig {
        location,
        directory: matches
            .value_of("directory")
            .map(String::from)
            .expect("directory missing"),
        decompress: matches.is_present("decompress"),
        insecure: matches.is_present("insecure"),
    }))
}

fn parse_list_stream(matches: &ArgMatches) -> Result<Config> {
    let stream_base_url = if let Some(base_url) = matches.value_of("stream-base-url") {
        Some(Url::parse(base_url).chain_err(|| "parsing stream base URL")?)
    } else {
        None
    };
    Ok(Config::ListStream(ListStreamConfig {
        stream_base_url,
        stream: matches
            .value_of("stream")
            .map(String::from)
            .expect("stream missing"),
    }))
}

fn parse_iso_embed(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::IsoEmbed(IsoEmbedConfig {
        input: matches
            .value_of("input")
            .map(String::from)
            .expect("input missing"),
        output: matches.value_of("output").map(String::from),
        ignition: matches.value_of("config").map(String::from),
        force: matches.is_present("force"),
    }))
}

fn parse_iso_show(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::IsoShow(IsoShowConfig {
        input: matches
            .value_of("input")
            .map(String::from)
            .expect("input missing"),
    }))
}

fn parse_iso_remove(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::IsoRemove(IsoRemoveConfig {
        input: matches
            .value_of("input")
            .map(String::from)
            .expect("input missing"),
        output: matches.value_of("output").map(String::from),
    }))
}

fn parse_real_rootdev<T: AsRef<str>>(s: Option<T>) -> Result<Option<OsmetRootBlkDevReal>> {
    if let Some(v) = s {
        let v = v.as_ref();
        let parts: Vec<_> = v.splitn(2, ',').collect();
        if parts.len() < 2 {
            bail!("Expected DEVICE,OFFSET-SECTORS but found {}", v);
        }
        let offset = parts[1].parse()?;
        Ok(Some(OsmetRootBlkDevReal {
            underlying_device: parts[0].to_string(),
            offset_sectors: offset,
        }))
    } else {
        Ok(None)
    }
}

fn parse_osmet_pack(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::OsmetPack(OsmetPackConfig {
        output: matches
            .value_of("output")
            .map(String::from)
            .expect("output missing"),
        device: matches
            .value_of("device")
            .map(String::from)
            .expect("device missing"),
        checksum: matches
            .value_of("checksum")
            .map(String::from)
            .expect("checksum missing"),
        description: matches
            .value_of("description")
            .map(String::from)
            .expect("description missing"),
        rootdev: parse_real_rootdev(matches.value_of("real-rootdev"))?,
        fast: matches.is_present("fast"),
    }))
}

fn parse_osmet_unpack(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::OsmetUnpack(OsmetUnpackConfig {
        repo: matches
            .value_of("repo")
            .map(String::from)
            .expect("repo missing"),
        osmet: matches
            .value_of("osmet")
            .map(String::from)
            .expect("osmet file missing"),
        device: matches
            .value_of("device")
            .map(String::from)
            .expect("device missing"),
    }))
}

fn parse_osmet_fiemap(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::OsmetFiemap(OsmetFiemapConfig {
        file: matches
            .value_of("file")
            .map(String::from)
            .expect("file missing"),
    }))
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
}
