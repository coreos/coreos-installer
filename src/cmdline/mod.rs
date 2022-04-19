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

use clap::{AppSettings, Parser};
use reqwest::Url;

mod install;
#[cfg(feature = "mangen")]
mod man;
mod serializer;
mod types;

pub use self::install::InstallConfig;
#[cfg(feature = "mangen")]
pub use self::man::*;
pub use self::types::*;

// Args are listed in --help in the order declared in these structs/enums.
// Please keep the entire help text to 80 columns.

/// Installer for Fedora CoreOS and RHEL CoreOS
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
    /// Generate man pages for coreos-installer
    #[cfg(feature = "mangen")]
    Man(PackManConfig),
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

#[cfg(feature = "mangen")]
#[derive(Debug, Parser)]
pub struct PackManConfig {
    /// Output directory
    #[clap(short = 'C', long, value_name = "path", default_value = ".")]
    pub directory: String,
}

#[cfg(test)]
mod test {
    use super::*;
    use clap::IntoApp;

    #[test]
    fn clap_app() {
        Cmd::command().debug_assert()
    }
}
