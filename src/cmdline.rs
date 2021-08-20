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

use anyhow::{anyhow, Error, Result};
use reqwest::Url;
use std::default::Default;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::string::ToString;
use structopt::clap::AppSettings;
use structopt::StructOpt;

use crate::io::IgnitionHash;
#[cfg(target_arch = "s390x")]
use crate::s390x::dasd_try_get_sector_size;

// Args are listed in --help in the order declared in these structs/enums.
// Please keep the entire help text to 80 columns.

#[derive(Debug, StructOpt)]
#[structopt(name = "coreos-installer")]
#[structopt(global_setting(AppSettings::ArgsNegateSubcommands))]
#[structopt(global_setting(AppSettings::DeriveDisplayOrder))]
#[structopt(global_setting(AppSettings::DisableHelpSubcommand))]
#[structopt(global_setting(AppSettings::UnifiedHelpMessage))]
#[structopt(global_setting(AppSettings::VersionlessSubcommands))]
pub enum Cmd {
    /// Install Fedora CoreOS or RHEL CoreOS
    Install(InstallConfig),
    /// Download a CoreOS image
    Download(DownloadConfig),
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
pub enum IsoCmd {
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
pub enum IsoIgnitionCmd {
    /// Embed an Ignition config in an ISO image
    Embed(IsoIgnitionEmbedConfig),
    /// Show the embedded Ignition config from an ISO image
    Show(IsoIgnitionShowConfig),
    /// Remove an existing embedded Ignition config from an ISO image
    Remove(IsoIgnitionRemoveConfig),
}

#[derive(Debug, StructOpt)]
pub enum IsoKargsCmd {
    /// Modify kernel args in an ISO image
    Modify(IsoKargsModifyConfig),
    /// Reset kernel args in an ISO image to defaults
    Reset(IsoKargsResetConfig),
    /// Show kernel args from an ISO image
    Show(IsoKargsShowConfig),
}

#[derive(Debug, StructOpt)]
pub enum OsmetCmd {
    /// Create osmet file from CoreOS block device
    Pack(OsmetPackConfig),
    /// Generate raw metal image from osmet file and OSTree repo
    Unpack(OsmetUnpackConfig),
    /// Print file extent mapping of specific file
    Fiemap(OsmetFiemapConfig),
}

#[derive(Debug, StructOpt)]
pub enum PxeCmd {
    /// Commands to manage a live PXE Ignition config
    Ignition(PxeIgnitionCmd),
}

#[derive(Debug, StructOpt)]
pub enum PxeIgnitionCmd {
    /// Wrap an Ignition config in an initrd image
    Wrap(PxeIgnitionWrapConfig),
    /// Show the wrapped Ignition config in an initrd image
    Unwrap(PxeIgnitionUnwrapConfig),
}

#[derive(Debug, StructOpt)]
pub struct InstallConfig {
    // ways to specify the image source
    /// Fedora CoreOS stream
    #[structopt(short, long, value_name = "name")]
    #[structopt(conflicts_with = "image-file", conflicts_with = "image-url")]
    pub stream: Option<String>,
    /// Manually specify the image URL
    #[structopt(short = "u", long, value_name = "URL")]
    #[structopt(conflicts_with = "stream", conflicts_with = "image-file")]
    pub image_url: Option<Url>,
    /// Manually specify a local image file
    #[structopt(short = "f", long, value_name = "path")]
    #[structopt(conflicts_with = "stream", conflicts_with = "image-url")]
    pub image_file: Option<String>,

    // postprocessing options
    /// Embed an Ignition config from a file
    // deprecated long name from <= 0.1.2
    #[structopt(short, long, alias = "ignition", value_name = "path")]
    #[structopt(conflicts_with = "ignition-url")]
    pub ignition_file: Option<String>,
    /// Embed an Ignition config from a URL
    #[structopt(short = "I", long, value_name = "URL")]
    #[structopt(conflicts_with = "ignition-file")]
    pub ignition_url: Option<Url>,
    /// Digest (type-value) of the Ignition config
    #[structopt(long, value_name = "digest")]
    pub ignition_hash: Option<IgnitionHash>,
    /// Override the Ignition platform ID
    #[structopt(short, long, value_name = "name")]
    pub platform: Option<String>,
    /// Additional kernel args for the first boot
    // This used to be for configuring networking from the cmdline, but it has
    // been obsoleted by the nicer `--copy-network` approach. We still need it
    // for now though. It's used at least by `coreos-installer.service`.
    #[structopt(long, hidden = true, value_name = "args")]
    pub firstboot_args: Option<String>,
    /// Append default kernel arg
    #[structopt(long, value_name = "arg", number_of_values = 1)]
    pub append_karg: Vec<String>,
    /// Delete default kernel arg
    #[structopt(long, value_name = "arg", number_of_values = 1)]
    pub delete_karg: Vec<String>,
    /// Copy network config from install environment
    #[structopt(short = "n", long)]
    pub copy_network: bool,
    /// For use with -n.
    #[structopt(long, value_name = "path", empty_values = false)]
    #[structopt(default_value = "/etc/NetworkManager/system-connections/")]
    // don't strip trailing .
    #[structopt(verbatim_doc_comment)]
    // so we can stay under 80 chars
    #[structopt(next_line_help(true))]
    pub network_dir: String,
    /// Save partitions with this label glob
    #[structopt(long, value_name = "lx")]
    // Allow argument multiple times, but one value each.  Allow "a,b" in
    // one argument.
    #[structopt(number_of_values = 1, require_delimiter = true)]
    pub save_partlabel: Vec<String>,
    /// Save partitions with this number or range
    #[structopt(long, value_name = "id")]
    // Allow argument multiple times, but one value each.  Allow "1-5,7" in
    // one argument.
    #[structopt(number_of_values = 1, require_delimiter = true)]
    // Allow ranges like "-2".
    #[structopt(allow_hyphen_values = true)]
    pub save_partindex: Vec<String>,

    // obscure options without short names
    /// Force offline installation
    #[structopt(long)]
    pub offline: bool,
    /// Skip signature verification
    #[structopt(long)]
    pub insecure: bool,
    /// Allow Ignition URL without HTTPS or hash
    #[structopt(long)]
    pub insecure_ignition: bool,
    /// Base URL for Fedora CoreOS stream metadata
    #[structopt(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
    /// Target CPU architecture
    #[structopt(long, default_value, value_name = "name")]
    pub architecture: Architecture,
    /// Don't clear partition table on error
    #[structopt(long)]
    pub preserve_on_error: bool,
    /// Fetch retries, or "infinite"
    #[structopt(long, value_name = "N", default_value)]
    pub fetch_retries: FetchRetries,

    // positional args
    /// Destination device
    pub device: String,
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

#[derive(Debug, StructOpt)]
pub struct DownloadConfig {
    /// Fedora CoreOS stream
    #[structopt(short, long, value_name = "name", default_value = "stable")]
    pub stream: String,
    /// Target CPU architecture
    #[structopt(long, value_name = "name", default_value)]
    pub architecture: Architecture,
    /// Fedora CoreOS platform name
    #[structopt(short, long, value_name = "name", default_value = "metal")]
    pub platform: String,
    /// Image format
    #[structopt(short, long, value_name = "name", default_value = "raw.xz")]
    pub format: String,
    /// Manually specify the image URL
    #[structopt(short = "u", long, value_name = "URL")]
    pub image_url: Option<Url>,
    /// Destination directory
    #[structopt(short = "C", long, value_name = "path", default_value = ".")]
    pub directory: String,
    /// Decompress image and don't save signature
    #[structopt(short, long)]
    pub decompress: bool,
    /// Skip signature verification
    #[structopt(long)]
    pub insecure: bool,
    /// Base URL for Fedora CoreOS stream metadata
    #[structopt(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
    /// Fetch retries, or "infinite"
    #[structopt(long, value_name = "N", default_value)]
    pub fetch_retries: FetchRetries,
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
pub struct IsoEmbedConfig {
    /// Ignition config to embed [default: stdin]
    #[structopt(short, long, value_name = "path")]
    pub config: Option<String>,
    /// Overwrite an existing embedded Ignition config
    #[structopt(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoShowConfig {
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoRemoveConfig {
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
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
    /// Show ISO header (for debugging/testing only)
    #[structopt(long, hidden = true)]
    pub header: bool,
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
