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

use anyhow::{Context, Result};
use clap::{AppSettings, Parser};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};
use std::default::Default;
use std::ffi::OsStr;
use std::fs::OpenOptions;

use crate::io::IgnitionHash;

mod serializer;
mod types;

pub use self::types::*;

// Args are listed in --help in the order declared in these structs/enums.
// Please keep the entire help text to 80 columns.

#[derive(Debug, Parser)]
#[clap(version)]
#[clap(global_setting(AppSettings::DeriveDisplayOrder))]
#[clap(args_conflicts_with_subcommands = true)]
#[clap(disable_help_subcommand = true)]
#[clap(help_expected = true)]
pub enum Cmd {
    /// Install Fedora CoreOS or RHEL CoreOS
    Install(InstallConfig),
    /// Download a CoreOS image
    Download(DownloadConfig),
    /// List available images in a Fedora CoreOS stream
    ListStream(ListStreamConfig),
    /// Commands to manage a CoreOS live ISO image
    #[clap(subcommand)]
    Iso(IsoCmd),
    /// Commands to manage a CoreOS live PXE image
    #[clap(subcommand)]
    Pxe(PxeCmd),
    /// Metadata packing commands used when building an OS image
    #[clap(subcommand)]
    Pack(PackCmd),
    /// Development commands (unstable)
    #[clap(subcommand)]
    Dev(DevCmd),
}

#[derive(Debug, Parser)]
pub enum IsoCmd {
    /// Embed an Ignition config in an ISO image
    // deprecated
    #[clap(hide = true)]
    Embed(IsoEmbedConfig),
    /// Show the embedded Ignition config from an ISO image
    // deprecated
    #[clap(hide = true)]
    Show(IsoShowConfig),
    /// Remove an existing embedded Ignition config from an ISO image
    // deprecated
    #[clap(hide = true)]
    Remove(IsoRemoveConfig),
    /// Customize a CoreOS live ISO image
    Customize(IsoCustomizeConfig),
    /// Embed an Ignition config in a CoreOS live ISO image
    #[clap(subcommand)]
    Ignition(IsoIgnitionCmd),
    /// Embed network settings in a CoreOS live ISO image
    #[clap(subcommand)]
    Network(IsoNetworkCmd),
    /// Modify kernel args in a CoreOS live ISO image
    #[clap(subcommand)]
    Kargs(IsoKargsCmd),
    /// Commands to extract files from a CoreOS live ISO image
    #[clap(subcommand)]
    Extract(IsoExtractCmd),
    /// Restore a CoreOS live ISO image to default settings
    Reset(IsoResetConfig),
}

#[derive(Debug, Parser)]
pub enum IsoIgnitionCmd {
    /// Embed an Ignition config in an ISO image
    Embed(IsoIgnitionEmbedConfig),
    /// Show the embedded Ignition config from an ISO image
    Show(IsoIgnitionShowConfig),
    /// Remove an existing embedded Ignition config from an ISO image
    Remove(IsoIgnitionRemoveConfig),
}

#[derive(Debug, Parser)]
pub enum IsoNetworkCmd {
    /// Embed network settings in an ISO image
    Embed(IsoNetworkEmbedConfig),
    /// Extract embedded network settings from an ISO image
    Extract(IsoNetworkExtractConfig),
    /// Remove existing network settings from an ISO image
    Remove(IsoNetworkRemoveConfig),
}

#[derive(Debug, Parser)]
pub enum IsoKargsCmd {
    /// Modify kernel args in an ISO image
    Modify(IsoKargsModifyConfig),
    /// Reset kernel args in an ISO image to defaults
    Reset(IsoKargsResetConfig),
    /// Show kernel args from an ISO image
    Show(IsoKargsShowConfig),
}

#[derive(Debug, Parser)]
pub enum IsoExtractCmd {
    /// Extract PXE files from an ISO image
    Pxe(IsoExtractPxeConfig),
    /// Extract a minimal ISO from a CoreOS live ISO image
    MinimalIso(IsoExtractMinimalIsoConfig),
}

#[derive(Debug, Parser)]
pub enum PxeCmd {
    /// Create a custom live PXE boot config
    Customize(PxeCustomizeConfig),
    /// Commands to manage a live PXE Ignition config
    #[clap(subcommand)]
    Ignition(PxeIgnitionCmd),
    /// Commands to manage live PXE network settings
    #[clap(subcommand)]
    Network(PxeNetworkCmd),
}

#[derive(Debug, Parser)]
pub enum PxeIgnitionCmd {
    /// Wrap an Ignition config in an initrd image
    Wrap(PxeIgnitionWrapConfig),
    /// Show the wrapped Ignition config in an initrd image
    Unwrap(PxeIgnitionUnwrapConfig),
}

#[derive(Debug, Parser)]
pub enum PxeNetworkCmd {
    /// Wrap network settings in an initrd image
    Wrap(PxeNetworkWrapConfig),
    /// Extract wrapped network settings from an initrd image
    Unwrap(PxeNetworkUnwrapConfig),
}

#[derive(Debug, Parser)]
// users shouldn't be interacting with this command normally
#[clap(hide = true)]
pub enum PackCmd {
    /// Create osmet file from CoreOS block device
    Osmet(PackOsmetConfig),
    /// Pack a minimal ISO into a CoreOS live ISO image
    MinimalIso(PackMinimalIsoConfig),
}

#[derive(Debug, Parser)]
// users shouldn't be interacting with this command normally
#[clap(hide = true)]
pub enum DevCmd {
    /// Commands to show metadata
    #[clap(subcommand)]
    Show(DevShowCmd),
    /// Commands to extract data
    #[clap(subcommand)]
    Extract(DevExtractCmd),
}

#[derive(Debug, Parser)]
pub enum DevShowCmd {
    /// Inspect the CoreOS live ISO image
    Iso(DevShowIsoConfig),
    /// Show the contents of an initrd image
    Initrd(DevShowInitrdConfig),
    /// Print file extent mapping of specific file
    Fiemap(DevShowFiemapConfig),
}

#[derive(Debug, Parser)]
pub enum DevExtractCmd {
    /// Generate raw metal image from osmet file and OSTree repo
    Osmet(DevExtractOsmetConfig),
    /// Extract the contents of an initrd image
    Initrd(DevExtractInitrdConfig),
}

const ADVANCED: &str = "ADVANCED OPTIONS";

// As a special case, this struct supports Serialize and Deserialize for
// config file parsing.  Here are the rules.  Build or test should fail if
// you break anything too badly.
// - Defaults cannot be specified using #[clap(default_value = "x")]
//   because serde won't see them otherwise.  Instead, use
//   #[clap(default_value_t)], implement Default, and derive PartialEq
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
#[derive(Debug, Default, Parser, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
#[clap(args_override_self = true)]
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
    #[clap(short, long, value_name = "path")]
    pub config_file: Vec<String>,

    // ways to specify the image source
    /// Fedora CoreOS stream
    ///
    /// The name of the Fedora CoreOS stream to install, such as "stable",
    /// "testing", or "next".
    #[clap(short, long, value_name = "name")]
    #[clap(conflicts_with = "image-file", conflicts_with = "image-url")]
    pub stream: Option<String>,
    /// Manually specify the image URL
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[clap(short = 'u', long, value_name = "URL")]
    #[clap(conflicts_with = "stream", conflicts_with = "image-file")]
    pub image_url: Option<Url>,
    /// Manually specify a local image file
    #[clap(short = 'f', long, value_name = "path")]
    #[clap(conflicts_with = "stream", conflicts_with = "image-url")]
    pub image_file: Option<String>,

    // postprocessing options
    /// Embed an Ignition config from a file
    // deprecated long name from <= 0.1.2
    #[clap(short, long, alias = "ignition", value_name = "path")]
    #[clap(conflicts_with = "ignition-url")]
    pub ignition_file: Option<String>,
    /// Embed an Ignition config from a URL
    ///
    /// Immediately fetch the Ignition config from the URL and embed it in
    /// the installed system.
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[clap(short = 'I', long, value_name = "URL")]
    #[clap(conflicts_with = "ignition-file")]
    pub ignition_url: Option<Url>,
    /// Digest (type-value) of the Ignition config
    ///
    /// Verify that the Ignition config matches the specified digest,
    /// formatted as <type>-<hexvalue>.  <type> can be sha256 or sha512.
    #[clap(long, value_name = "digest")]
    pub ignition_hash: Option<IgnitionHash>,
    /// Target CPU architecture
    ///
    /// Create an install disk for a different CPU architecture than the
    /// host.
    #[serde(skip_serializing_if = "is_default")]
    #[clap(short, long, default_value_t, value_name = "name")]
    pub architecture: DefaultedString<Architecture>,
    /// Override the Ignition platform ID
    ///
    /// Install a system that will run on the specified cloud or
    /// virtualization platform, such as "vmware".
    #[clap(short, long, value_name = "name")]
    pub platform: Option<String>,
    /// Additional kernel args for the first boot
    // This used to be for configuring networking from the cmdline, but it has
    // been obsoleted by the nicer `--copy-network` approach. We still need it
    // for now though. It's used at least by `coreos-installer.service`.
    #[serde(skip)]
    #[clap(long, hide = true, value_name = "args")]
    pub firstboot_args: Option<String>,
    /// Append default kernel arg
    ///
    /// Add a kernel argument to the installed system.
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, value_name = "arg")]
    pub append_karg: Vec<String>,
    /// Delete default kernel arg
    ///
    /// Delete a default kernel argument from the installed system.
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, value_name = "arg")]
    pub delete_karg: Vec<String>,
    /// Copy network config from install environment
    ///
    /// Copy NetworkManager keyfiles from the install environment to the
    /// installed system.
    #[serde(skip_serializing_if = "is_default")]
    #[clap(short = 'n', long)]
    pub copy_network: bool,
    /// Override NetworkManager keyfile dir for -n
    ///
    /// Specify the path to NetworkManager keyfiles to be copied with
    /// --copy-network.
    ///
    /// [default: /etc/NetworkManager/system-connections/]
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, value_name = "path", default_value_t)]
    // showing the default converts every option to multiline help
    #[clap(hide_default_value = true)]
    pub network_dir: DefaultedString<NetworkDir>,
    /// Save partitions with this label glob
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, value_name = "lx")]
    // Allow argument multiple times, but one value each.  Allow "a,b" in
    // one argument.
    #[clap(number_of_values = 1, require_value_delimiter = true)]
    #[clap(value_delimiter = ',')]
    pub save_partlabel: Vec<String>,
    /// Save partitions with this number or range
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, value_name = "id")]
    // Allow argument multiple times, but one value each.  Allow "1-5,7" in
    // one argument.
    #[clap(number_of_values = 1, require_value_delimiter = true)]
    #[clap(value_delimiter = ',')]
    // Allow ranges like "-2".
    #[clap(allow_hyphen_values = true)]
    pub save_partindex: Vec<String>,

    // obscure options without short names
    /// Force offline installation
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, help_heading = ADVANCED)]
    pub offline: bool,
    /// Skip signature verification
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, help_heading = ADVANCED)]
    pub insecure: bool,
    /// Allow Ignition URL without HTTPS or hash
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, help_heading = ADVANCED)]
    pub insecure_ignition: bool,
    /// Base URL for CoreOS stream metadata
    ///
    /// Override the base URL for fetching CoreOS stream metadata.
    /// The default is "https://builds.coreos.fedoraproject.org/streams/".
    #[serde_as(as = "Option<DisplayFromStr>")]
    #[clap(long, value_name = "URL", help_heading = ADVANCED)]
    pub stream_base_url: Option<Url>,
    /// Don't clear partition table on error
    ///
    /// If installation fails, coreos-installer normally clears the
    /// destination's partition table to prevent booting from invalid
    /// boot media.  Skip clearing the partition table as a debugging aid.
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, help_heading = ADVANCED)]
    pub preserve_on_error: bool,
    /// Fetch retries, or "infinite"
    ///
    /// Number of times to retry network fetches, or the string "infinite"
    /// to retry indefinitely.
    #[serde(skip_serializing_if = "is_default")]
    #[clap(long, value_name = "N", default_value_t, help_heading = ADVANCED)]
    pub fetch_retries: FetchRetries,

    // positional args
    /// Destination device
    ///
    /// Path to the device node for the destination disk.  The beginning of
    /// the device will be overwritten without further confirmation.
    #[clap(required_unless_present = "config-file")]
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
        match Cmd::try_parse_from(
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

#[derive(Debug, Parser)]
pub struct DownloadConfig {
    /// Fedora CoreOS stream
    #[clap(short, long, value_name = "name", default_value = "stable")]
    pub stream: String,
    /// Target CPU architecture
    #[clap(short, long, value_name = "name", default_value_t)]
    pub architecture: DefaultedString<Architecture>,
    /// Fedora CoreOS platform name
    #[clap(short, long, value_name = "name", default_value = "metal")]
    pub platform: String,
    /// Image format
    #[clap(short, long, value_name = "name", default_value = "raw.xz")]
    pub format: String,
    /// Manually specify the image URL
    #[clap(short = 'u', long, value_name = "URL")]
    pub image_url: Option<Url>,
    /// Destination directory
    #[clap(short = 'C', long, value_name = "path", default_value = ".")]
    pub directory: String,
    /// Decompress image and don't save signature
    #[clap(short, long)]
    pub decompress: bool,
    /// Skip signature verification
    #[clap(long)]
    pub insecure: bool,
    /// Base URL for Fedora CoreOS stream metadata
    #[clap(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
    /// Fetch retries, or "infinite"
    #[clap(long, value_name = "N", default_value_t)]
    pub fetch_retries: FetchRetries,
}

#[derive(Debug, Parser)]
pub struct ListStreamConfig {
    /// Fedora CoreOS stream
    #[clap(short, long, value_name = "name", default_value = "stable")]
    pub stream: String,
    /// Base URL for Fedora CoreOS stream metadata
    #[clap(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
}

#[derive(Debug, Parser)]
pub struct CommonCustomizeConfig {
    /// Ignition config fragment for dest sys
    ///
    /// Automatically run installer and merge the specified Ignition config
    /// into the config for the destination system.
    #[clap(long, value_name = "path")]
    pub dest_ignition: Vec<String>,
    /// Install destination device
    ///
    /// Automatically run installer, installing to the specified destination
    /// device.  The resulting boot media will overwrite the destination
    /// device without confirmation.
    #[clap(long, value_name = "path")]
    pub dest_device: Option<String>,
    /// Destination kernel argument to append
    ///
    /// Automatically run installer, adding the specified kernel argument
    /// for every boot of the destination system.
    #[clap(long, value_name = "arg")]
    pub dest_karg_append: Vec<String>,
    /// Destination kernel argument to delete
    ///
    /// Automatically run installer, deleting the specified kernel argument
    /// for every boot of the destination system.
    #[clap(long, value_name = "arg")]
    pub dest_karg_delete: Vec<String>,
    /// NetworkManager keyfile for live & dest
    ///
    /// Configure networking using the specified NetworkManager keyfile.
    /// Network settings will be applied in the live environment, including
    /// when Ignition is run.  If installer is enabled via additional options,
    /// network settings will also be applied in the destination system,
    /// including when Ignition is run.
    #[clap(long, value_name = "path")]
    pub network_keyfile: Vec<String>,
    /// Ignition PEM CA bundle for live & dest
    ///
    /// Specify additional TLS certificate authorities to be trusted by
    /// Ignition, in PEM format.  Authorities will be trusted by Ignition
    /// in the live environment and, if installer is enabled via additional
    /// options, in the destination system.
    #[clap(long, value_name = "path")]
    pub ignition_ca: Vec<String>,
    /// Script to run before installation
    ///
    /// If installer is run at boot, run this script before installation.
    /// If the script fails, the live environment will stop at an emergency
    /// shell.
    #[clap(long, value_name = "path")]
    pub pre_install: Vec<String>,
    /// Script to run after installation
    ///
    /// If installer is run at boot, run this script after installation.
    /// If the script fails, the live environment will stop at an emergency
    /// shell.
    #[clap(long, value_name = "path")]
    pub post_install: Vec<String>,
    /// Installer config file
    ///
    /// Automatically run coreos-installer and apply the specified installer
    /// config file.  Config files are applied in the order that they are
    /// specified.
    #[clap(long, value_name = "path")]
    pub installer_config: Vec<String>,
    /// Ignition config fragment for live env
    ///
    /// Merge the specified Ignition config into the config for the live
    /// environment.
    #[clap(long, value_name = "path")]
    pub live_ignition: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct IsoCustomizeConfig {
    // Customizations
    #[clap(flatten)]
    pub common: CommonCustomizeConfig,
    /// Live kernel argument to append
    ///
    /// Kernel argument to append to boots of the live environment.
    #[clap(long, value_name = "arg")]
    pub live_karg_append: Vec<String>,
    /// Live kernel argument to delete
    ///
    /// Kernel argument to delete from boots of the live environment.
    #[clap(long, value_name = "arg")]
    pub live_karg_delete: Vec<String>,
    /// Live kernel argument to replace
    ///
    /// Kernel argument to replace for boots of the live environment, in the
    /// form key=old=new.  For a default argument "a=b", specifying
    /// "--live-karg-replace a=b=c" will produce the argument "a=c".
    #[clap(long, value_name = "k=o=n")]
    pub live_karg_replace: Vec<String>,

    // I/O configuration
    /// Overwrite existing customizations
    #[clap(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoEmbedConfig {
    /// Ignition config to embed [default: stdin]
    #[clap(short, long, value_name = "path")]
    pub config: Option<String>,
    /// Overwrite an existing embedded Ignition config
    #[clap(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoShowConfig {
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoRemoveConfig {
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoIgnitionEmbedConfig {
    /// Overwrite an existing Ignition config
    #[clap(short, long)]
    pub force: bool,
    /// Ignition config to embed [default: stdin]
    #[clap(short, long, value_name = "path")]
    pub ignition_file: Option<String>,
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoIgnitionShowConfig {
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoIgnitionRemoveConfig {
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoNetworkEmbedConfig {
    /// NetworkManager keyfile to embed
    // Required option. :-(  In future we might support other configuration
    // sources.
    #[clap(short, long, required = true, value_name = "path")]
    pub keyfile: Vec<String>,
    /// Overwrite existing network settings
    #[clap(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoNetworkExtractConfig {
    /// Extract to directory instead of stdout
    #[clap(short = 'C', long, value_name = "path")]
    pub directory: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoNetworkRemoveConfig {
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoKargsModifyConfig {
    /// Kernel argument to append
    #[clap(short, long, value_name = "KARG")]
    pub append: Vec<String>,
    /// Kernel argument to delete
    #[clap(short, long, value_name = "KARG")]
    pub delete: Vec<String>,
    /// Kernel argument to replace
    #[clap(short, long, value_name = "KARG=OLDVAL=NEWVAL")]
    pub replace: Vec<String>,
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "PATH")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoKargsResetConfig {
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "PATH")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoKargsShowConfig {
    /// Show default kernel args
    #[clap(short, long)]
    pub default: bool,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct DevShowIsoConfig {
    /// Show Ignition embed area parameters
    #[clap(long, conflicts_with = "kargs")]
    pub ignition: bool,
    /// Show kargs embed area parameters
    #[clap(long, conflicts_with = "ignition")]
    pub kargs: bool,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoExtractPxeConfig {
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
    /// Output directory
    #[clap(short, long, value_name = "PATH", default_value = ".")]
    pub output_dir: String,
}

#[derive(Debug, Parser)]
pub struct IsoExtractMinimalIsoConfig {
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
    /// Extract rootfs image as well
    #[clap(long, value_name = "PATH")]
    pub output_rootfs: Option<String>,
    /// Minimal ISO output file
    #[clap(value_name = "OUTPUT_ISO", default_value = "-")]
    pub output: String,
    /// Inject rootfs URL karg into minimal ISO
    #[clap(long, value_name = "URL")]
    pub rootfs_url: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PackMinimalIsoConfig {
    /// ISO image
    #[clap(value_name = "FULL_ISO")]
    pub full: String,
    /// Minimal ISO image
    #[clap(value_name = "MINIMAL_ISO")]
    pub minimal: String,
    /// Delete minimal ISO after packing
    #[clap(long)]
    pub consume: bool,
}

#[derive(Debug, Parser)]
pub struct IsoResetConfig {
    /// Write ISO to a new output file
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[clap(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
// default usage line lists all mandatory options and so exceeds 80 characters
#[clap(override_usage = "coreos-installer pack osmet [OPTIONS]")]
pub struct PackOsmetConfig {
    /// Path to osmet file to write
    // could output to stdout if missing?
    #[clap(long, required = true, value_name = "FILE")]
    pub output: String,
    /// Expected SHA256 of block device
    // XXX: rebase on top of
    // https://github.com/coreos/coreos-installer/pull/178 and use the same
    // type-digest format
    #[clap(long, required = true, value_name = "SHA256")]
    pub checksum: String,
    /// Description of OS
    #[clap(long, required = true, value_name = "TEXT")]
    pub description: String,
    /// Use worse compression, for development builds
    #[clap(long)]
    pub fast: bool,
    /// Source device
    #[clap(value_name = "DEV")]
    pub device: String,
}

#[derive(Debug, Parser)]
pub struct DevExtractOsmetConfig {
    /// osmet file
    #[clap(long, required = true, value_name = "PATH")]
    pub osmet: String,
    /// OSTree repo
    #[clap(value_name = "PATH")]
    pub repo: String,
    /// Destination device
    #[clap(value_name = "DEV")]
    pub device: String,
}

#[derive(Debug, Parser)]
pub struct DevShowFiemapConfig {
    /// File to map
    #[clap(value_name = "PATH")]
    pub file: String,
}

#[derive(Debug, Parser)]
pub struct PxeCustomizeConfig {
    // Customizations
    #[clap(flatten)]
    pub common: CommonCustomizeConfig,

    // I/O configuration
    /// Output file
    #[clap(short, long, value_name = "path")]
    pub output: String,
    /// CoreOS live initramfs image
    #[clap(value_name = "path")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct PxeIgnitionWrapConfig {
    /// Ignition config to wrap [default: stdin]
    #[clap(short, long, value_name = "path")]
    pub ignition_file: Option<String>,
    /// Write to a file instead of stdout
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PxeIgnitionUnwrapConfig {
    /// initrd image [default: stdin]
    #[clap(value_name = "initrd")]
    pub input: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PxeNetworkWrapConfig {
    /// NetworkManager keyfile to embed
    // Required option. :-(  In future we might support other configuration
    // sources.
    #[clap(short, long, required = true, value_name = "path")]
    pub keyfile: Vec<String>,
    /// Write to a file instead of stdout
    #[clap(short, long, value_name = "path")]
    pub output: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PxeNetworkUnwrapConfig {
    /// Extract to directory instead of stdout
    #[clap(short = 'C', long, value_name = "path")]
    pub directory: Option<String>,
    /// initrd image [default: stdin]
    #[clap(value_name = "initrd")]
    pub input: Option<String>,
}

#[derive(Debug, Parser)]
pub struct DevShowInitrdConfig {
    /// initrd image ("-" for stdin)
    #[clap(value_name = "initrd")]
    pub input: String,
    /// Files or globs to list
    #[clap(value_name = "glob")]
    pub filter: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct DevExtractInitrdConfig {
    /// Output directory
    #[clap(short = 'C', long, value_name = "path", default_value = ".")]
    pub directory: String,
    /// List extracted contents
    #[clap(short, long)]
    pub verbose: bool,
    /// initrd image ("-" for stdin)
    #[clap(value_name = "initrd")]
    pub input: String,
    /// Files or globs to list
    #[clap(value_name = "glob")]
    pub filter: Vec<String>,
}

#[cfg(test)]
mod test {
    use super::*;
    use clap::IntoApp;
    use std::io::Write;
    use std::num::NonZeroU32;
    use std::str::FromStr;
    use tempfile::NamedTempFile;

    #[test]
    fn clap_app() {
        Cmd::command().debug_assert()
    }

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
