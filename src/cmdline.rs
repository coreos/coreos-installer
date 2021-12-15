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

// We don't care about the size of enum variants and don't want to box them
#![allow(clippy::large_enum_variant)]

use anyhow::{anyhow, Context, Error, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_with::{
    serde_as, skip_serializing_none, DeserializeFromStr, DisplayFromStr, SerializeDisplay,
};
use std::default::Default;
use std::ffi::OsStr;
use std::fmt;
use std::fs::OpenOptions;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::str::FromStr;
use structopt::clap::AppSettings;
use structopt::StructOpt;

use crate::io::IgnitionHash;

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
    /// Customize a CoreOS live ISO image
    Customize(IsoCustomizeConfig),
    /// Embed an Ignition config in a CoreOS live ISO image
    Ignition(IsoIgnitionCmd),
    /// Embed network settings in a CoreOS live ISO image
    Network(IsoNetworkCmd),
    /// Modify kernel args in a CoreOS live ISO image
    Kargs(IsoKargsCmd),
    /// Inspect the CoreOS live ISO image
    // for testing and debugging purposes only
    #[structopt(setting(AppSettings::Hidden))]
    Inspect(IsoInspectConfig),
    /// Commands to extract files from a CoreOS live ISO image
    Extract(IsoExtractCmd),
    /// Restore a CoreOS live ISO image to default settings
    Reset(IsoResetConfig),
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
pub enum IsoNetworkCmd {
    /// Embed network settings in an ISO image
    Embed(IsoNetworkEmbedConfig),
    /// Extract embedded network settings from an ISO image
    Extract(IsoNetworkExtractConfig),
    /// Remove existing network settings from an ISO image
    Remove(IsoNetworkRemoveConfig),
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
pub enum IsoExtractCmd {
    /// Extract PXE files from an ISO image
    Pxe(IsoExtractPxeConfig),
    /// Extract a minimal ISO from a CoreOS live ISO image
    MinimalIso(IsoExtractMinimalIsoConfig),
    // This doesn't really make sense under `extract`, but it's hidden and conceptually feels
    // cleaner being alongside `coreos-installer iso extract minimal-iso`.
    /// Pack a minimal ISO into a CoreOS live ISO image
    #[structopt(setting(AppSettings::Hidden))]
    PackMinimalIso(IsoExtractPackMinimalIsoConfig),
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
    /// Create a custom live PXE boot config
    Customize(PxeCustomizeConfig),
    /// Commands to manage a live PXE Ignition config
    Ignition(PxeIgnitionCmd),
    /// Commands to manage live PXE network settings
    Network(PxeNetworkCmd),
}

#[derive(Debug, StructOpt)]
pub enum PxeIgnitionCmd {
    /// Wrap an Ignition config in an initrd image
    Wrap(PxeIgnitionWrapConfig),
    /// Show the wrapped Ignition config in an initrd image
    Unwrap(PxeIgnitionUnwrapConfig),
}

#[derive(Debug, StructOpt)]
pub enum PxeNetworkCmd {
    /// Wrap network settings in an initrd image
    Wrap(PxeNetworkWrapConfig),
    /// Extract wrapped network settings from an initrd image
    Unwrap(PxeNetworkUnwrapConfig),
}

// As a special case, this struct supports Serialize and Deserialize for
// config file parsing.  Here are the rules.  Build or test should fail if
// you break anything too badly.
// - Defaults cannot be specified using #[structopt(default_value = "x")]
//   because serde won't see them otherwise.  Instead, use
//   #[structopt(default_value)], implement Default, and derive PartialEq
//   for the type.  (For string-typed defaults, you can use
//   DefaultedString<T> where T is a custom type implementing
//   DefaultString.)
// - Add #[serde(skip_serializing_if = "is_default")] for all fields that
//   are not Option<T>.
// - Custom types used in fields should implement Display and FromStr, then
//   implement Serialize/Deserialize by deriving SerializeDisplay/
//   DeserializeFromStr.
// - reqwest::Url doesn't implement Serialize/Deserialize, but does implement
//   Display and FromStr, so use #[serde_as(as = "Option<DisplayFromStr>")].
// - Use #[serde(skip)] for any option that shouldn't be supported in config
//   files.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Default, StructOpt, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
#[structopt(global_setting(AppSettings::AllArgsOverrideSelf))]
pub struct InstallConfig {
    /// YAML config file with install options
    ///
    /// Load additional config options from the specified YAML config file.
    /// Later config files override earlier ones, and command-line options
    /// override config files.
    ///
    /// Config file keys are long option names without the leading "--".
    /// Values are strings for non-repeatable options, arrays of strings for
    /// repeatable options, and "true" for flags.  The destination device
    /// can be specified with the "dest-device" key.
    #[serde(skip)]
    #[structopt(short, long, value_name = "path", number_of_values = 1)]
    pub config_file: Vec<String>,

    // ways to specify the image source
    /// Fedora CoreOS stream
    ///
    /// The name of the Fedora CoreOS stream to install, such as "stable",
    /// "testing", or "next".
    #[structopt(short, long, value_name = "name")]
    #[structopt(conflicts_with = "image-file", conflicts_with = "image-url")]
    pub stream: Option<String>,
    /// Manually specify the image URL
    #[serde_as(as = "Option<DisplayFromStr>")]
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
    ///
    /// Immediately fetch the Ignition config from the URL and embed it in
    /// the installed system.
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[structopt(short = "I", long, value_name = "URL")]
    #[structopt(conflicts_with = "ignition-file")]
    pub ignition_url: Option<Url>,
    /// Digest (type-value) of the Ignition config
    ///
    /// Verify that the Ignition config matches the specified digest,
    /// formatted as <type>-<hexvalue>.  <type> can be sha256 or sha512.
    #[structopt(long, value_name = "digest")]
    pub ignition_hash: Option<IgnitionHash>,
    /// Target CPU architecture
    ///
    /// Create an install disk for a different CPU architecture than the
    /// host.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(short, long, default_value, value_name = "name")]
    pub architecture: DefaultedString<Architecture>,
    /// Override the Ignition platform ID
    ///
    /// Install a system that will run on the specified cloud or
    /// virtualization platform, such as "vmware".
    #[structopt(short, long, value_name = "name")]
    pub platform: Option<String>,
    /// Additional kernel args for the first boot
    // This used to be for configuring networking from the cmdline, but it has
    // been obsoleted by the nicer `--copy-network` approach. We still need it
    // for now though. It's used at least by `coreos-installer.service`.
    #[serde(skip)]
    #[structopt(long, hidden = true, value_name = "args")]
    pub firstboot_args: Option<String>,
    /// Append default kernel arg
    ///
    /// Add a kernel argument to the installed system.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long, value_name = "arg", number_of_values = 1)]
    pub append_karg: Vec<String>,
    /// Delete default kernel arg
    ///
    /// Delete a default kernel argument from the installed system.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long, value_name = "arg", number_of_values = 1)]
    pub delete_karg: Vec<String>,
    /// Copy network config from install environment
    ///
    /// Copy NetworkManager keyfiles from the install environment to the
    /// installed system.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(short = "n", long)]
    pub copy_network: bool,
    /// For use with -n
    ///
    /// Specify the path to NetworkManager keyfiles to be copied with
    /// --copy-network.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long, value_name = "path", default_value)]
    // so we can stay under 80 chars
    #[structopt(next_line_help(true))]
    pub network_dir: DefaultedString<NetworkDir>,
    /// Save partitions with this label glob
    #[structopt(long, value_name = "lx")]
    // Allow argument multiple times, but one value each.  Allow "a,b" in
    // one argument.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(number_of_values = 1, require_delimiter = true)]
    pub save_partlabel: Vec<String>,
    /// Save partitions with this number or range
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long, value_name = "id")]
    // Allow argument multiple times, but one value each.  Allow "1-5,7" in
    // one argument.
    #[structopt(number_of_values = 1, require_delimiter = true)]
    // Allow ranges like "-2".
    #[structopt(allow_hyphen_values = true)]
    pub save_partindex: Vec<String>,

    // obscure options without short names
    /// Force offline installation
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long)]
    pub offline: bool,
    /// Skip signature verification
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long)]
    pub insecure: bool,
    /// Allow Ignition URL without HTTPS or hash
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long)]
    pub insecure_ignition: bool,
    /// Base URL for CoreOS stream metadata
    ///
    /// Override the base URL for fetching CoreOS stream metadata.
    /// The default is "https://builds.coreos.fedoraproject.org/streams/".
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[structopt(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
    /// Don't clear partition table on error
    ///
    /// If installation fails, coreos-installer normally clears the
    /// destination's partition table to prevent booting from invalid
    /// boot media.  Skip clearing the partition table as a debugging aid.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long)]
    pub preserve_on_error: bool,
    /// Fetch retries, or "infinite"
    ///
    /// Number of times to retry network fetches, or the string "infinite"
    /// to retry indefinitely.
    #[serde(skip_serializing_if = "is_default")]
    #[structopt(long, value_name = "N", default_value)]
    pub fetch_retries: FetchRetries,

    // positional args
    /// Destination device
    ///
    /// Path to the device node for the destination disk.  The beginning of
    /// the device will be overwritten without further confirmation.
    #[structopt(required_unless = "config-file")]
    pub dest_device: Option<String>,
}

impl InstallConfig {
    pub fn expand_config_files(self) -> Result<Self> {
        if self.config_file.is_empty() {
            return Ok(self);
        }

        let args = self
            .config_file
            .iter()
            .map(|path| {
                serde_yaml::from_reader::<_, InstallConfig>(
                    OpenOptions::new()
                        .read(true)
                        .open(path)
                        .with_context(|| format!("opening config file {}", path))?,
                )
                .with_context(|| format!("parsing config file {}", path))?
                .to_args()
                .with_context(|| format!("serializing config file {}", path))
            })
            .collect::<Result<Vec<Vec<_>>>>()?
            .into_iter()
            .flatten()
            .chain(
                self.to_args()
                    .context("serializing command-line arguments")?,
            )
            .collect::<Vec<_>>();

        println!("Running with arguments: {}", args.join(" "));
        Self::from_args(&args)
    }

    fn from_args<T: AsRef<OsStr>>(args: &[T]) -> Result<Self> {
        match Cmd::from_iter_safe(
            vec![
                std::env::args_os().next().expect("no program name"),
                "install".into(),
            ]
            .into_iter()
            .chain(args.iter().map(<_>::into)),
        )
        .context("reprocessing command-line arguments")?
        {
            Cmd::Install(c) => Ok(c),
            _ => unreachable!(),
        }
    }

    fn to_args(&self) -> Result<Vec<String>> {
        serializer::to_args(self)
    }
}

#[derive(Debug, DeserializeFromStr, SerializeDisplay, Clone, Copy, PartialEq, Eq)]
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
    #[structopt(short, long, value_name = "name", default_value)]
    pub architecture: DefaultedString<Architecture>,
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
pub struct IsoCustomizeConfig {
    /// Overwrite existing customizations
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
pub struct IsoNetworkEmbedConfig {
    /// NetworkManager keyfile to embed
    // Required option. :-(  In future we might support other configuration
    // sources.
    #[structopt(short, long, required = true, value_name = "path")]
    #[structopt(number_of_values = 1)]
    pub keyfile: Vec<String>,
    /// Overwrite existing network settings
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
pub struct IsoNetworkExtractConfig {
    /// Extract to directory instead of stdout
    #[structopt(short = "C", long, value_name = "path")]
    pub directory: Option<String>,
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoNetworkRemoveConfig {
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
pub struct IsoInspectConfig {
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoExtractPxeConfig {
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
    /// Output directory
    #[structopt(short, long, value_name = "PATH", default_value = ".")]
    pub output_dir: String,
}

#[derive(Debug, StructOpt)]
pub struct IsoExtractMinimalIsoConfig {
    /// ISO image
    #[structopt(value_name = "ISO")]
    pub input: String,
    /// Extract rootfs image as well
    #[structopt(long, value_name = "PATH")]
    pub output_rootfs: Option<String>,
    /// Minimal ISO output file
    #[structopt(value_name = "OUTPUT_ISO", default_value = "-")]
    pub output: String,
    /// Inject rootfs URL karg into minimal ISO
    #[structopt(long, value_name = "URL")]
    pub rootfs_url: Option<String>,
}

#[derive(Debug, StructOpt)]
pub struct IsoExtractPackMinimalIsoConfig {
    /// ISO image
    #[structopt(value_name = "FULL_ISO")]
    pub full: String,
    /// Minimal ISO image
    #[structopt(value_name = "MINIMAL_ISO")]
    pub minimal: String,
    /// Delete minimal ISO after packing
    #[structopt(long)]
    pub consume: bool,
}

#[derive(Debug, StructOpt)]
pub struct IsoResetConfig {
    /// Write ISO to a new output file
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
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
pub struct PxeCustomizeConfig {
    /// Output file
    #[structopt(short, long, value_name = "path")]
    pub output: String,
    /// CoreOS live initramfs image
    #[structopt(value_name = "path")]
    pub input: String,
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
    /// initrd image [default: stdin]
    #[structopt(value_name = "initrd")]
    pub input: Option<String>,
}

#[derive(Debug, StructOpt)]
pub struct PxeNetworkWrapConfig {
    /// NetworkManager keyfile to embed
    // Required option. :-(  In future we might support other configuration
    // sources.
    #[structopt(short, long, required = true, value_name = "path")]
    #[structopt(number_of_values = 1)]
    pub keyfile: Vec<String>,
    /// Write to a file instead of stdout
    #[structopt(short, long, value_name = "path")]
    pub output: Option<String>,
}

#[derive(Debug, StructOpt)]
pub struct PxeNetworkUnwrapConfig {
    /// Extract to directory instead of stdout
    #[structopt(short = "C", long, value_name = "path")]
    pub directory: Option<String>,
    /// initrd image [default: stdin]
    #[structopt(value_name = "initrd")]
    pub input: Option<String>,
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

impl fmt::Display for FetchRetries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "0"),
            Self::Finite(n) => write!(f, "{}", n),
            Self::Infinite => write!(f, "infinite"),
        }
    }
}

impl Default for FetchRetries {
    fn default() -> Self {
        Self::None
    }
}

/// A String wrapper that takes a parameterized type defining the default
/// value of the String.
#[derive(Debug, PartialEq, Eq)]
pub struct DefaultedString<S: DefaultString> {
    value: String,
    default: PhantomData<S>,
}

impl<S: DefaultString> DefaultedString<S> {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl<S: DefaultString> FromStr for DefaultedString<S> {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            value: s.to_string(),
            default: PhantomData,
        })
    }
}

impl<S: DefaultString> fmt::Display for DefaultedString<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl<S: DefaultString> Default for DefaultedString<S> {
    fn default() -> Self {
        Self {
            value: S::default(),
            default: PhantomData,
        }
    }
}

// SerializeDisplay derive apparently doesn't work with parameterized types
impl<S: DefaultString> Serialize for DefaultedString<S> {
    fn serialize<R>(&self, serializer: R) -> Result<R::Ok, R::Error>
    where
        R: serde::ser::Serializer,
    {
        serializer.serialize_str(&self.value)
    }
}

// DeserializeFromStr derive apparently doesn't work with parameterized types
impl<'de, S: DefaultString> Deserialize<'de> for DefaultedString<S> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        Ok(Self {
            value: String::deserialize(deserializer)?,
            default: PhantomData,
        })
    }
}

/// A default value for a DefaultedString.
pub trait DefaultString {
    fn default() -> String;
}

/// A default string of `uname -m`.
#[derive(Debug, PartialEq, Eq)]
pub struct Architecture {}
impl DefaultString for Architecture {
    fn default() -> String {
        nix::sys::utsname::uname().machine().to_string()
    }
}

/// The default path to NetworkManager connection files.
#[derive(Debug, PartialEq, Eq)]
pub struct NetworkDir {}
impl DefaultString for NetworkDir {
    fn default() -> String {
        "/etc/NetworkManager/system-connections/".into()
    }
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

mod serializer {
    use anyhow::Context;
    use serde::{ser, Serialize};
    use structopt::StructOpt;

    pub fn to_args<T>(value: &T) -> anyhow::Result<Vec<String>>
    where
        T: Serialize + StructOpt,
    {
        // We need to be able to find out whether a field is an --option
        // or a positional argument.  structopt and clap don't provide an
        // API for this, and we don't want to implement a proc macro because
        // those have to go in a separate crate.  Get the subcommand help
        // text and grep it.  :-(
        let mut help = Vec::new();
        T::clap()
            .write_long_help(&mut help)
            .context("reading subcommand help text")?;

        let mut serializer = Serializer {
            help_text: String::from_utf8(help).context("decoding subcommand help text")?,
            output: Vec::new(),
            field_stack: Vec::new(),
        };
        value.serialize(&mut serializer)?;
        Ok(serializer.output)
    }

    struct Serializer {
        help_text: String,
        output: Vec<String>,
        field_stack: Vec<Option<&'static str>>,
    }

    impl Serializer {
        fn push_field(&mut self, name: &'static str) {
            let field = if self.help_text.contains(&format!(" --{} ", name)) {
                Some(name)
            } else {
                // don't serialize to --option
                None
            };
            self.field_stack.push(field);
        }

        fn pop_field(&mut self) {
            self.field_stack.pop();
        }

        fn output_option(&mut self) {
            match &self.field_stack[self.field_stack.len() - 1] {
                None => (),
                Some(name) => {
                    let option = format!("--{}", name);
                    self.output_argument(option);
                }
            }
        }

        fn output_argument<T: ToString>(&mut self, arg: T) {
            self.output.push(arg.to_string());
        }
    }

    // Enormous pile of boilerplate covering every basic type.  We only need
    // to handle a few:
    // - The containing struct => walk each field, tracking option names
    // - Sequences => add an option argument before each Vec entry
    // - Options => serialize the wrapped value if Some
    // - Bools => add option argument only if true
    // - String/number primitives => add option argument, then value
    // https://serde.rs/impl-serializer.html
    // https://docs.serde.rs/serde/trait.Serializer.html
    impl<'a> ser::Serializer for &'a mut Serializer {
        type Ok = ();
        type Error = SerializeError;
        type SerializeSeq = Self;
        type SerializeStruct = Self;
        type SerializeMap = ser::Impossible<Self::Ok, Self::Error>;
        type SerializeStructVariant = ser::Impossible<Self::Ok, Self::Error>;
        type SerializeTuple = ser::Impossible<Self::Ok, Self::Error>;
        type SerializeTupleStruct = ser::Impossible<Self::Ok, Self::Error>;
        type SerializeTupleVariant = ser::Impossible<Self::Ok, Self::Error>;

        fn serialize_bool(self, v: bool) -> Result<()> {
            if v {
                self.output_option();
            }
            Ok(())
        }

        fn serialize_i8(self, v: i8) -> Result<()> {
            self.serialize_i64(i64::from(v))
        }

        fn serialize_i16(self, v: i16) -> Result<()> {
            self.serialize_i64(i64::from(v))
        }

        fn serialize_i32(self, v: i32) -> Result<()> {
            self.serialize_i64(i64::from(v))
        }

        fn serialize_i64(self, v: i64) -> Result<()> {
            self.output_option();
            self.output_argument(v);
            Ok(())
        }

        fn serialize_u8(self, v: u8) -> Result<()> {
            self.serialize_u64(u64::from(v))
        }

        fn serialize_u16(self, v: u16) -> Result<()> {
            self.serialize_u64(u64::from(v))
        }

        fn serialize_u32(self, v: u32) -> Result<()> {
            self.serialize_u64(u64::from(v))
        }

        fn serialize_u64(self, v: u64) -> Result<()> {
            self.output_option();
            self.output_argument(v);
            Ok(())
        }

        fn serialize_f32(self, v: f32) -> Result<()> {
            self.serialize_f64(f64::from(v))
        }

        fn serialize_f64(self, v: f64) -> Result<()> {
            self.output_option();
            self.output_argument(v);
            Ok(())
        }

        fn serialize_char(self, v: char) -> Result<()> {
            self.serialize_str(&v.to_string())
        }

        fn serialize_str(self, v: &str) -> Result<()> {
            self.output_option();
            self.output_argument(v);
            Ok(())
        }

        fn serialize_bytes(self, _v: &[u8]) -> Result<()> {
            unimplemented!()
        }

        fn serialize_none(self) -> Result<()> {
            Ok(())
        }

        fn serialize_some<T>(self, value: &T) -> Result<()>
        where
            T: ?Sized + Serialize,
        {
            value.serialize(self)
        }

        // Anonymous value containing no data
        fn serialize_unit(self) -> Result<()> {
            Ok(())
        }

        // Named value containing no data
        fn serialize_unit_struct(self, _name: &'static str) -> Result<()> {
            Ok(())
        }

        // Unit enum variant
        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
        ) -> Result<()> {
            unimplemented!()
        }

        fn serialize_newtype_struct<T>(self, _name: &'static str, _value: &T) -> Result<()>
        where
            T: ?Sized + Serialize,
        {
            unimplemented!()
        }

        fn serialize_newtype_variant<T>(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _value: &T,
        ) -> Result<()>
        where
            T: ?Sized + Serialize,
        {
            unimplemented!()
        }

        fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
            // no setup to be done
            Ok(self)
        }

        fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple> {
            unimplemented!();
        }

        fn serialize_tuple_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleStruct> {
            unimplemented!();
        }

        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleVariant> {
            unimplemented!();
        }

        fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
            unimplemented!();
        }

        fn serialize_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStruct> {
            // no setup to be done
            Ok(self)
        }

        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStructVariant> {
            unimplemented!();
        }
    }

    impl<'a> ser::SerializeSeq for &'a mut Serializer {
        type Ok = ();
        type Error = SerializeError;

        fn serialize_element<T>(&mut self, value: &T) -> Result<()>
        where
            T: ?Sized + Serialize,
        {
            value.serialize(&mut **self)
        }

        fn end(self) -> Result<()> {
            Ok(())
        }
    }

    impl<'a> ser::SerializeStruct for &'a mut Serializer {
        type Ok = ();
        type Error = SerializeError;

        fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
        where
            T: ?Sized + Serialize,
        {
            self.push_field(key);
            let ret = value.serialize(&mut **self);
            self.pop_field();
            ret
        }

        fn end(self) -> Result<()> {
            Ok(())
        }
    }

    pub type Result<T> = std::result::Result<T, SerializeError>;

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct SerializeError(String);

    impl ser::Error for SerializeError {
        fn custom<T: ToString>(msg: T) -> Self {
            SerializeError(msg.to_string())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Check that full InstallConfig serializes as expected
    #[test]
    fn serialize_full_install_config() {
        let config = InstallConfig {
            // skipped
            config_file: vec!["a".into(), "b".into()],
            stream: Some("c".into()),
            image_url: Some(Url::parse("http://example.com/d").unwrap()),
            image_file: Some("e".into()),
            ignition_file: Some("f".into()),
            ignition_url: Some(Url::parse("http://example.com/g").unwrap()),
            ignition_hash: Some(
                IgnitionHash::from_str(
                    "sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                )
                .unwrap(),
            ),
            architecture: DefaultedString::<Architecture>::from_str("h").unwrap(),
            platform: Some("i".into()),
            // skipped
            firstboot_args: Some("j".into()),
            append_karg: vec!["k".into(), "l".into()],
            delete_karg: vec!["m".into(), "n".into()],
            copy_network: true,
            network_dir: DefaultedString::<NetworkDir>::from_str("o").unwrap(),
            save_partlabel: vec!["p".into(), "q".into()],
            save_partindex: vec!["r".into(), "s".into()],
            offline: true,
            insecure: true,
            insecure_ignition: true,
            stream_base_url: Some(Url::parse("http://example.com/t").unwrap()),
            preserve_on_error: true,
            fetch_retries: FetchRetries::from_str("3").unwrap(),
            dest_device: Some("u".into()),
        };
        let expected = vec![
            "--stream",
            "c",
            "--image-url",
            "http://example.com/d",
            "--image-file",
            "e",
            "--ignition-file",
            "f",
            "--ignition-url",
            "http://example.com/g",
            "--ignition-hash",
            "sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "--architecture",
            "h",
            "--platform",
            "i",
            "--append-karg",
            "k",
            "--append-karg",
            "l",
            "--delete-karg",
            "m",
            "--delete-karg",
            "n",
            "--copy-network",
            "--network-dir",
            "o",
            "--save-partlabel",
            "p",
            "--save-partlabel",
            "q",
            "--save-partindex",
            "r",
            "--save-partindex",
            "s",
            "--offline",
            "--insecure",
            "--insecure-ignition",
            "--stream-base-url",
            "http://example.com/t",
            "--preserve-on-error",
            "--fetch-retries",
            "3",
            "u",
        ];
        assert_eq!(config.to_args().unwrap(), expected);
    }

    /// Test that full config file deserializes as expected
    #[test]
    fn parse_full_install_config_file() {
        let mut f = NamedTempFile::new().unwrap();
        f.as_file_mut()
            .write_all(
                r#"
image-url: http://example.com/d
ignition-url: http://example.com/g
ignition-hash: sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
architecture: h
platform: i
append-karg: [k, l]
delete-karg: [m, n]
copy-network: true
network-dir: o
save-partlabel: [p, q]
save-partindex: [r, s]
offline: true
insecure: true
insecure-ignition: true
stream-base-url: http://example.com/t
preserve-on-error: true
fetch-retries: 3
dest-device: u
"#
                .as_bytes(),
            )
            .unwrap();
        let expected = InstallConfig {
            // skipped
            config_file: Vec::new(),
            // conflict
            stream: None,
            image_url: Some(Url::parse("http://example.com/d").unwrap()),
            // conflict
            image_file: None,
            // conflict
            ignition_file: None,
            ignition_url: Some(Url::parse("http://example.com/g").unwrap()),
            ignition_hash: Some(
                IgnitionHash::from_str(
                    "sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                )
                .unwrap(),
            ),
            architecture: DefaultedString::<Architecture>::from_str("h").unwrap(),
            platform: Some("i".into()),
            // skipped
            firstboot_args: None,
            append_karg: vec!["k".into(), "l".into()],
            delete_karg: vec!["m".into(), "n".into()],
            copy_network: true,
            network_dir: DefaultedString::<NetworkDir>::from_str("o").unwrap(),
            save_partlabel: vec!["p".into(), "q".into()],
            save_partindex: vec!["r".into(), "s".into()],
            offline: true,
            insecure: true,
            insecure_ignition: true,
            stream_base_url: Some(Url::parse("http://example.com/t").unwrap()),
            preserve_on_error: true,
            fetch_retries: FetchRetries::from_str("3").unwrap(),
            dest_device: Some("u".into()),
        };
        let config = InstallConfig::from_args(&["--config-file", f.path().to_str().unwrap()])
            .unwrap()
            .expand_config_files()
            .unwrap();
        assert_eq!(expected, config);
    }

    /// Check that default InstallConfig serializes to empty arg list
    #[test]
    fn serialize_default_install_config_args() {
        let config = InstallConfig::default();
        let expected: Vec<String> = Vec::new();
        assert_eq!(config.to_args().unwrap(), expected);
    }

    /// Check that default InstallConfig serializes to empty YAML doc
    #[test]
    fn serialize_default_install_config_yaml() {
        let config = InstallConfig::default();
        assert_eq!(serde_yaml::to_string(&config).unwrap(), "---\n{}\n");
    }

    /// Check that minimal install config file serializes to minimal arg list
    #[test]
    fn serialize_empty_install_config_file() {
        let config: InstallConfig = serde_yaml::from_str("dest-device: foo").unwrap();
        assert_eq!(config.to_args().unwrap(), vec!["foo"]);
    }

    /// Check that empty command line serializes to empty arg list
    #[test]
    fn serialize_empty_command_line() {
        let expected = ["/dev/missing"];
        let config = InstallConfig::from_args(&expected).unwrap();
        assert_eq!(config.to_args().unwrap(), expected);
    }

    /// Test multiple config files overlapping with command-line arguments
    #[test]
    fn install_config_file_overlapping_field() {
        let mut f1 = NamedTempFile::new().unwrap();
        f1.as_file_mut()
            .write_all(b"append-karg: [a, b]\nfetch-retries: 1")
            .unwrap();
        let mut f2 = NamedTempFile::new().unwrap();
        f2.as_file_mut()
            .write_all(b"append-karg: [c, d]\nfetch-retries: 2\ndest-device: /dev/missing")
            .unwrap();
        let config = InstallConfig::from_args(&[
            "--append-karg",
            "e",
            "--fetch-retries",
            "0",
            "--config-file",
            f2.path().to_str().unwrap(),
            "--config-file",
            f1.path().to_str().unwrap(),
            "--append-karg",
            "f",
            "--fetch-retries",
            "3",
        ])
        .unwrap()
        .expand_config_files()
        .unwrap();
        assert_eq!(config.append_karg, ["c", "d", "a", "b", "e", "f"]);
        assert_eq!(
            config.fetch_retries,
            FetchRetries::Finite(NonZeroU32::new(3).unwrap())
        );

        // multiple target devices are not allowed
        InstallConfig::from_args(&[
            "--config-file",
            f2.path().to_str().unwrap(),
            "/dev/also-missing",
        ])
        .unwrap()
        .expand_config_files()
        .unwrap_err();
    }
}
