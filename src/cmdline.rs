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

use anyhow::{anyhow, bail, Context, Error, Result};
use reqwest::Url;
use std::default::Default;
use std::fs::{File, OpenOptions};
use std::num::NonZeroU32;
use std::path::Path;
use std::str::FromStr;
use std::string::ToString;
use structopt::clap::AppSettings;
use structopt::StructOpt;

use crate::blockdev::*;
use crate::download::*;
use crate::io::IgnitionHash;
#[cfg(target_arch = "s390x")]
use crate::s390x::dasd_try_get_sector_size;
use crate::source::*;

// Args are listed in --help in the order declared in these structs/enums.
// Please keep the entire help text to 80 columns.

// Exported, flattened subcommand enum with postprocessed configs
pub enum Config {
    Install(InstallConfig),
    Download(DownloadConfig),
    ListStream(ListStreamConfig),
    IsoEmbed(IsoIgnitionEmbedConfig),
    IsoShow(IsoIgnitionShowConfig),
    IsoRemove(IsoIgnitionRemoveConfig),
    IsoIgnitionEmbed(IsoIgnitionEmbedConfig),
    IsoIgnitionShow(IsoIgnitionShowConfig),
    IsoIgnitionRemove(IsoIgnitionRemoveConfig),
    IsoKargsModify(IsoKargsModifyConfig),
    IsoKargsReset(IsoKargsResetConfig),
    IsoKargsShow(IsoKargsShowConfig),
    OsmetFiemap(OsmetFiemapConfig),
    OsmetPack(OsmetPackConfig),
    OsmetUnpack(OsmetUnpackConfig),
    PxeIgnitionWrap(PxeIgnitionWrapConfig),
    PxeIgnitionUnwrap(PxeIgnitionUnwrapConfig),
}

#[derive(Debug, StructOpt)]
#[structopt(name = "coreos-installer")]
#[structopt(global_setting(AppSettings::ArgsNegateSubcommands))]
#[structopt(global_setting(AppSettings::DeriveDisplayOrder))]
#[structopt(global_setting(AppSettings::DisableHelpSubcommand))]
#[structopt(global_setting(AppSettings::UnifiedHelpMessage))]
#[structopt(global_setting(AppSettings::VersionlessSubcommands))]
enum Cmd {
    /// Install Fedora CoreOS or RHEL CoreOS
    Install(InstallCmd),
    /// Download a CoreOS image
    Download(DownloadCmd),
    /// List available images in a Fedora CoreOS stream
    ListStream(ListStreamConfig),
    /// Commands to manage a CoreOS live ISO image
    Iso(IsoCmd),
    /// Efficient CoreOS metal disk image packing using OSTree commits
    // users shouldn't be interacting with this command normally
    #[structopt(setting(AppSettings::Hidden))]
    Osmet(OsmetCmd),
    /// Commands to manage a CoreOS live PXE image
    Pxe(PxeCmd),
}

#[derive(Debug, StructOpt)]
enum IsoCmd {
    /// Embed an Ignition config in an ISO image
    // deprecated
    #[structopt(setting(AppSettings::Hidden))]
    Embed(IsoEmbedConfig),
    /// Show the embedded Ignition config from an ISO image
    // deprecated
    #[structopt(setting(AppSettings::Hidden))]
    Show(IsoShowConfig),
    /// Remove an existing embedded Ignition config from an ISO image
    // deprecated
    #[structopt(setting(AppSettings::Hidden))]
    Remove(IsoRemoveConfig),
    /// Embed an Ignition config in a CoreOS live ISO image
    Ignition(IsoIgnitionCmd),
    /// Modify kernel args in a CoreOS live ISO image
    Kargs(IsoKargsCmd),
}

#[derive(Debug, StructOpt)]
enum IsoIgnitionCmd {
    /// Embed an Ignition config in an ISO image
    Embed(IsoIgnitionEmbedConfig),
    /// Show the embedded Ignition config from an ISO image
    Show(IsoIgnitionShowConfig),
    /// Remove an existing embedded Ignition config from an ISO image
    Remove(IsoIgnitionRemoveConfig),
}

#[derive(Debug, StructOpt)]
enum IsoKargsCmd {
    /// Modify kernel args in an ISO image
    Modify(IsoKargsModifyConfig),
    /// Reset kernel args in an ISO image to defaults
    Reset(IsoKargsResetConfig),
    /// Show kernel args from an ISO image
    Show(IsoKargsShowConfig),
}

#[derive(Debug, StructOpt)]
enum OsmetCmd {
    /// Create osmet file from CoreOS block device
    Pack(OsmetPackConfig),
    /// Generate raw metal image from osmet file and OSTree repo
    Unpack(OsmetUnpackConfig),
    /// Print file extent mapping of specific file
    Fiemap(OsmetFiemapConfig),
}

#[derive(Debug, StructOpt)]
enum PxeCmd {
    /// Commands to manage a live PXE Ignition config
    Ignition(PxeIgnitionCmd),
}

#[derive(Debug, StructOpt)]
enum PxeIgnitionCmd {
    /// Wrap an Ignition config in an initrd image
    Wrap(PxeIgnitionWrapConfig),
    /// Show the wrapped Ignition config in an initrd image
    Unwrap(PxeIgnitionUnwrapConfig),
}

// Raw command-line arguments before postprocessing into InstallConfig.
#[derive(Debug, StructOpt)]
struct InstallCmd {
    // ways to specify the image source
    /// Fedora CoreOS stream
    #[structopt(short, long, value_name = "name")]
    #[structopt(conflicts_with = "image-file", conflicts_with = "image-url")]
    stream: Option<String>,
    /// Manually specify the image URL
    #[structopt(short = "u", long, value_name = "URL")]
    #[structopt(conflicts_with = "stream", conflicts_with = "image-file")]
    image_url: Option<Url>,
    /// Manually specify a local image file
    #[structopt(short = "f", long, value_name = "path")]
    #[structopt(conflicts_with = "stream", conflicts_with = "image-url")]
    image_file: Option<String>,

    // postprocessing options
    /// Embed an Ignition config from a file
    // deprecated long name from <= 0.1.2
    #[structopt(short, long, alias = "ignition", value_name = "path")]
    #[structopt(conflicts_with = "ignition-url")]
    ignition_file: Option<String>,
    /// Embed an Ignition config from a URL
    #[structopt(short = "I", long, value_name = "URL")]
    #[structopt(conflicts_with = "ignition-file")]
    ignition_url: Option<Url>,
    /// Digest (type-value) of the Ignition config
    #[structopt(long, value_name = "digest")]
    ignition_hash: Option<IgnitionHash>,
    /// Override the Ignition platform ID
    #[structopt(short, long, value_name = "name")]
    platform: Option<String>,
    /// Additional kernel args for the first boot
    // This used to be for configuring networking from the cmdline, but it has
    // been obsoleted by the nicer `--copy-network` approach. We still need it
    // for now though. It's used at least by `coreos-installer.service`.
    #[structopt(long, hidden = true, value_name = "args")]
    firstboot_args: Option<String>,
    /// Append default kernel arg
    #[structopt(long, value_name = "arg", number_of_values = 1)]
    append_karg: Vec<String>,
    /// Delete default kernel arg
    #[structopt(long, value_name = "arg", number_of_values = 1)]
    delete_karg: Vec<String>,
    /// Copy network config from install environment
    #[structopt(short = "n", long)]
    copy_network: bool,
    /// For use with -n.
    #[structopt(long, value_name = "path", empty_values = false)]
    #[structopt(default_value = "/etc/NetworkManager/system-connections/")]
    // don't strip trailing .
    #[structopt(verbatim_doc_comment)]
    // so we can stay under 80 chars
    #[structopt(next_line_help(true))]
    network_dir: String,
    /// Save partitions with this label glob
    #[structopt(long, value_name = "lx")]
    // Allow argument multiple times, but one value each.  Allow "a,b" in
    // one argument.
    #[structopt(number_of_values = 1, require_delimiter = true)]
    save_partlabel: Vec<String>,
    /// Save partitions with this number or range
    #[structopt(long, value_name = "id")]
    // Allow argument multiple times, but one value each.  Allow "1-5,7" in
    // one argument.
    #[structopt(number_of_values = 1, require_delimiter = true)]
    // Allow ranges like "-2".
    #[structopt(allow_hyphen_values = true)]
    save_partindex: Vec<String>,

    // obscure options without short names
    /// Force offline installation
    #[structopt(long)]
    offline: bool,
    /// Skip signature verification
    #[structopt(long)]
    insecure: bool,
    /// Allow Ignition URL without HTTPS or hash
    #[structopt(long)]
    insecure_ignition: bool,
    /// Base URL for Fedora CoreOS stream metadata
    #[structopt(long, value_name = "URL")]
    stream_base_url: Option<Url>,
    /// Target CPU architecture
    #[structopt(long, default_value, value_name = "name")]
    architecture: Architecture,
    /// Don't clear partition table on error
    #[structopt(long)]
    preserve_on_error: bool,
    /// Fetch retries, or "infinite"
    #[structopt(long, value_name = "N", default_value)]
    fetch_retries: FetchRetries,

    // positional args
    /// Destination device
    device: String,
}

pub struct InstallConfig {
    pub device: String,
    pub location: Box<dyn ImageLocation>,
    pub ignition: Option<File>,
    pub ignition_hash: Option<IgnitionHash>,
    pub platform: Option<String>,
    pub firstboot_kargs: Option<String>,
    pub append_kargs: Vec<String>,
    pub delete_kargs: Vec<String>,
    pub insecure: bool,
    pub preserve_on_error: bool,
    pub network_config: Option<String>,
    pub save_partitions: Vec<PartitionFilter>,
    pub fetch_retries: FetchRetries,
}

#[derive(Debug, Clone, Copy)]
pub enum FetchRetries {
    Infinite,
    Finite(NonZeroU32),
    None,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PartitionFilter {
    Label(glob::Pattern),
    Index(Option<NonZeroU32>, Option<NonZeroU32>),
}

// Raw command-line arguments before postprocessing into DownloadConfig.
#[derive(Debug, StructOpt)]
struct DownloadCmd {
    /// Fedora CoreOS stream
    #[structopt(short, long, value_name = "name", default_value = "stable")]
    stream: String,
    /// Target CPU architecture
    #[structopt(long, value_name = "name", default_value)]
    architecture: Architecture,
    /// Fedora CoreOS platform name
    #[structopt(short, long, value_name = "name", default_value = "metal")]
    platform: String,
    /// Image format
    #[structopt(short, long, value_name = "name", default_value = "raw.xz")]
    format: String,
    /// Manually specify the image URL
    #[structopt(short = "u", long, value_name = "URL")]
    image_url: Option<Url>,
    /// Destination directory
    #[structopt(short = "C", long, value_name = "path", default_value = ".")]
    directory: String,
    /// Decompress image and don't save signature
    #[structopt(short, long)]
    decompress: bool,
    /// Skip signature verification
    #[structopt(long)]
    insecure: bool,
    /// Base URL for Fedora CoreOS stream metadata
    #[structopt(long, value_name = "URL")]
    stream_base_url: Option<Url>,
    /// Fetch retries, or "infinite"
    #[structopt(long, value_name = "N", default_value)]
    fetch_retries: FetchRetries,
}

pub struct DownloadConfig {
    pub location: Box<dyn ImageLocation>,
    pub directory: String,
    pub decompress: bool,
    pub insecure: bool,
}

#[derive(Debug, StructOpt)]
pub struct ListStreamConfig {
    /// Fedora CoreOS stream
    #[structopt(short, long, value_name = "name", default_value = "stable")]
    pub stream: String,
    /// Base URL for Fedora CoreOS stream metadata
    #[structopt(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
}

#[derive(Debug, StructOpt)]
struct IsoEmbedConfig {
    /// Ignition config to embed [default: stdin]
    #[structopt(short, long, value_name = "path")]
    config: Option<String>,
    /// Overwrite an existing embedded Ignition config
    #[structopt(short, long)]
    force: bool,
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    input: String,
}

#[derive(Debug, StructOpt)]
struct IsoShowConfig {
    /// ISO image
    #[structopt(value_name = "ISO")]
    input: String,
}

#[derive(Debug, StructOpt)]
struct IsoRemoveConfig {
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoIgnitionEmbedConfig {
    /// Overwrite an existing Ignition config
    #[structopt(short, long)]
    pub force: bool,
    /// Ignition config to embed [default: stdin]
    #[structopt(short, long, value_name = "path")]
    pub ignition_file: Option<String>,
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoIgnitionShowConfig {
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoIgnitionRemoveConfig {
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoKargsModifyConfig {
    /// Kernel argument to append
    #[structopt(short, long, number_of_values = 1, value_name = "KARG")]
    pub append: Vec<String>,
    /// Kernel argument to delete
    #[structopt(short, long, number_of_values = 1, value_name = "KARG")]
    pub delete: Vec<String>,
    /// Kernel argument to replace
    #[structopt(short, long, number_of_values = 1, value_name = "KARG=OLDVAL=NEWVAL")]
    pub replace: Vec<String>,
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "PATH")]
    pub output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoKargsResetConfig {
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "PATH")]
    pub output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoKargsShowConfig {
    /// Show default kernel args
    #[structopt(short, long)]
    pub default: bool,
    /// Show ISO header (for debugging/testing only)
    #[structopt(long, hidden = true)]
    pub header: bool,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct OsmetPackConfig {
    /// Path to osmet file to write
    // could output to stdout if missing?
    #[structopt(long, required = true, value_name = "FILE")]
    pub output: String,
    /// Expected SHA256 of block device
    // XXX: rebase on top of
    // https://github.com/coreos/coreos-installer/pull/178 and use the same
    // type-digest format
    #[structopt(long, required = true, value_name = "SHA256")]
    pub checksum: String,
    /// Description of OS
    #[structopt(long, required = true, value_name = "TEXT")]
    pub description: String,
    /// Use worse compression, for development builds
    #[structopt(long)]
    pub fast: bool,
    /// Source device
    #[structopt(value_name = "DEV")]
    pub device: String,
}

#[derive(Debug, StructOpt)]
pub struct OsmetUnpackConfig {
    /// osmet file
    #[structopt(long, required = true, value_name = "PATH")]
    pub osmet: String,
    /// OSTree repo
    #[structopt(value_name = "PATH")]
    pub repo: String,
    /// Destination device
    #[structopt(value_name = "DEV")]
    pub device: String,
}

#[derive(Debug, StructOpt)]
pub struct OsmetFiemapConfig {
    /// File to map
    #[structopt(value_name = "PATH")]
    pub file: String,
}

#[derive(Debug, StructOpt)]
pub struct PxeIgnitionWrapConfig {
    /// Ignition config to wrap [default: stdin]
    #[structopt(short, long, value_name = "path")]
    pub ignition_file: Option<String>,
    /// Write to a file instead of stdout
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
}

#[derive(Debug, StructOpt)]
pub struct PxeIgnitionUnwrapConfig {
    /// initrd image
    #[structopt(value_name = "initrd")]
    pub input: String,
}

/// Parse command-line arguments.
pub fn parse_args() -> Result<Config> {
    Ok(match Cmd::from_args() {
        Cmd::Install(config) => Config::Install(parse_install(config)?),
        Cmd::Download(config) => Config::Download(parse_download(config)?),
        Cmd::ListStream(config) => Config::ListStream(config),
        Cmd::Iso(config) => match config {
            IsoCmd::Embed(config) => Config::IsoEmbed(config.into()),
            IsoCmd::Show(config) => Config::IsoShow(config.into()),
            IsoCmd::Remove(config) => Config::IsoRemove(config.into()),
            IsoCmd::Ignition(config) => match config {
                IsoIgnitionCmd::Embed(config) => Config::IsoIgnitionEmbed(config),
                IsoIgnitionCmd::Show(config) => Config::IsoIgnitionShow(config),
                IsoIgnitionCmd::Remove(config) => Config::IsoIgnitionRemove(config),
            },
            IsoCmd::Kargs(config) => match config {
                IsoKargsCmd::Modify(config) => Config::IsoKargsModify(config),
                IsoKargsCmd::Reset(config) => Config::IsoKargsReset(config),
                IsoKargsCmd::Show(config) => Config::IsoKargsShow(config),
            },
        },
        Cmd::Osmet(config) => match config {
            OsmetCmd::Pack(config) => Config::OsmetPack(config),
            OsmetCmd::Unpack(config) => Config::OsmetUnpack(config),
            OsmetCmd::Fiemap(config) => Config::OsmetFiemap(config),
        },
        Cmd::Pxe(config) => match config {
            PxeCmd::Ignition(config) => match config {
                PxeIgnitionCmd::Wrap(config) => Config::PxeIgnitionWrap(config),
                PxeIgnitionCmd::Unwrap(config) => Config::PxeIgnitionUnwrap(config),
            },
        },
    })
}

fn parse_install(cmd: InstallCmd) -> Result<InstallConfig> {
    // Uninitialized ECKD DASD's blocksize is 512, but after formatting
    // it changes to the recommended 4096
    // https://bugzilla.redhat.com/show_bug.cgi?id=1905159
    #[allow(clippy::match_bool, clippy::match_single_binding)]
    let sector_size = match is_dasd(&cmd.device, None)
        .with_context(|| format!("checking whether {} is an IBM DASD disk", &cmd.device))?
    {
        #[cfg(target_arch = "s390x")]
        true => dasd_try_get_sector_size(&cmd.device).transpose(),
        _ => None,
    };
    let sector_size = sector_size
        .unwrap_or_else(|| get_sector_size_for_path(Path::new(&cmd.device)))
        .with_context(|| format!("getting sector size of {}", &cmd.device))?
        .get();

    let location: Box<dyn ImageLocation> = if let Some(image_file) = cmd.image_file {
        Box::new(FileLocation::new(&image_file))
    } else if let Some(image_url) = cmd.image_url {
        Box::new(UrlLocation::new(&image_url, cmd.fetch_retries))
    } else if cmd.offline {
        match OsmetLocation::new(cmd.architecture.as_str(), sector_size)? {
            Some(osmet) => Box::new(osmet),
            None => bail!("cannot perform offline install; metadata missing"),
        }
    } else {
        // For now, using --stream automatically will cause a download. In the future, we could
        // opportunistically use osmet if the version and stream match an osmet file/the live ISO.

        let maybe_osmet = if cmd.stream.is_some() {
            None
        } else {
            OsmetLocation::new(cmd.architecture.as_str(), sector_size)?
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
                        n, &cmd.device
                    );
                    "raw.xz"
                }
            };
            Box::new(StreamLocation::new(
                cmd.stream.as_deref().unwrap_or("stable"),
                cmd.architecture.as_str(),
                "metal",
                format,
                cmd.stream_base_url.as_ref(),
                cmd.fetch_retries,
            )?)
        }
    };

    let ignition = if let Some(file) = cmd.ignition_file {
        Some(
            OpenOptions::new()
                .read(true)
                .open(&file)
                .with_context(|| format!("opening source Ignition config {}", file))?,
        )
    } else if let Some(url) = cmd.ignition_url {
        if url.scheme() == "http" {
            if cmd.ignition_hash.is_none() && !cmd.insecure_ignition {
                bail!("refusing to fetch Ignition config over HTTP without --ignition-hash or --insecure-ignition");
            }
        } else if url.scheme() != "https" {
            bail!("unknown protocol for URL '{}'", url);
        }
        Some(
            download_to_tempfile(&url, cmd.fetch_retries)
                .with_context(|| format!("downloading source Ignition config {}", url))?,
        )
    } else {
        None
    };

    // and report it to the user
    eprintln!("{}", location);

    // If the user requested us to copy networking config by passing
    // -n or --copy-network then copy networking config from the
    // directory defined by --network-dir.
    let network_config = if cmd.copy_network {
        Some(cmd.network_dir)
    } else {
        None
    };

    // build configuration
    Ok(InstallConfig {
        device: cmd.device,
        location,
        ignition,
        fetch_retries: cmd.fetch_retries,
        ignition_hash: cmd.ignition_hash,
        platform: cmd.platform,
        firstboot_kargs: cmd.firstboot_args,
        append_kargs: cmd.append_karg,
        delete_kargs: cmd.delete_karg,
        insecure: cmd.insecure,
        preserve_on_error: cmd.preserve_on_error,
        network_config,
        save_partitions: parse_partition_filters(
            &cmd.save_partlabel
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>(),
            &cmd.save_partindex
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>(),
        )?,
    })
}

impl FromStr for FetchRetries {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "infinite" => Ok(Self::Infinite),
            num => num
                .parse::<u32>()
                .map(|num| NonZeroU32::new(num).map(Self::Finite).unwrap_or(Self::None))
                .map_err(|e| anyhow!(e)),
        }
    }
}

impl ToString for FetchRetries {
    fn to_string(&self) -> String {
        match self {
            Self::None => "0".into(),
            Self::Finite(n) => n.to_string(),
            Self::Infinite => "infinite".into(),
        }
    }
}

impl Default for FetchRetries {
    fn default() -> Self {
        Self::None
    }
}

fn parse_partition_filters(labels: &[&str], indexes: &[&str]) -> Result<Vec<PartitionFilter>> {
    use PartitionFilter::*;
    let mut filters: Vec<PartitionFilter> = Vec::new();

    // partition label globs
    for glob in labels {
        let filter = Label(
            glob::Pattern::new(glob)
                .with_context(|| format!("couldn't parse label glob '{}'", glob))?,
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
                        .with_context(|| format!("couldn't parse partition index '{}'", i))?,
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

fn parse_download(cmd: DownloadCmd) -> Result<DownloadConfig> {
    // Build image location.  Ideally we'd use conflicts_with (and an
    // ArgGroup for streams), but that doesn't play well with default
    // arguments, so we manually prioritize modes.
    let location: Box<dyn ImageLocation> = if let Some(image_url) = cmd.image_url {
        Box::new(UrlLocation::new(&image_url, cmd.fetch_retries))
    } else {
        Box::new(StreamLocation::new(
            &cmd.stream,
            cmd.architecture.as_str(),
            &cmd.platform,
            &cmd.format,
            cmd.stream_base_url.as_ref(),
            cmd.fetch_retries,
        )?)
    };

    // build configuration
    Ok(DownloadConfig {
        location,
        directory: cmd.directory,
        decompress: cmd.decompress,
        insecure: cmd.insecure,
    })
}

impl From<IsoEmbedConfig> for IsoIgnitionEmbedConfig {
    fn from(config: IsoEmbedConfig) -> Self {
        Self {
            force: config.force,
            ignition_file: config.config,
            output: config.output,
            input: config.input,
        }
    }
}

impl From<IsoShowConfig> for IsoIgnitionShowConfig {
    fn from(config: IsoShowConfig) -> Self {
        Self {
            input: config.input,
        }
    }
}

impl From<IsoRemoveConfig> for IsoIgnitionRemoveConfig {
    fn from(config: IsoRemoveConfig) -> Self {
        Self {
            output: config.output,
            input: config.input,
        }
    }
}

// A String wrapper with a default of `uname -m`.
#[derive(Debug)]
pub struct Architecture(String);

impl Architecture {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Architecture {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl ToString for Architecture {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

impl Default for Architecture {
    fn default() -> Self {
        Architecture(nix::sys::utsname::uname().machine().to_string())
    }
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
