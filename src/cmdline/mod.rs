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

use clap::Parser;
use reqwest::Url;

mod console;
#[cfg(feature = "docgen")]
mod doc;
mod install;
mod serializer;
mod types;

pub use self::console::*;
#[cfg(feature = "docgen")]
pub use self::doc::*;
pub use self::install::InstallConfig;
pub use self::types::*;

// Args are listed in --help in the order declared in these structs/enums.
// Please keep the entire help text to 80 columns.

/// Installer for Fedora CoreOS and RHEL CoreOS
#[derive(Debug, Parser)]
#[command(version)]
#[command(args_conflicts_with_subcommands = true)]
#[command(disable_help_subcommand = true)]
#[command(help_expected = true)]
pub enum Cmd {
    /// Install Fedora CoreOS or RHEL CoreOS
    Install(InstallConfig),
    /// Download a CoreOS image
    Download(DownloadConfig),
    /// List available images in a Fedora CoreOS stream
    ListStream(ListStreamConfig),
    /// Commands to manage a CoreOS live ISO image
    #[command(subcommand)]
    Iso(IsoCmd),
    /// Commands to manage a CoreOS live PXE image
    #[command(subcommand)]
    Pxe(PxeCmd),
    /// Metadata packing commands used when building an OS image
    #[command(subcommand)]
    Pack(PackCmd),
    /// Development commands (unstable)
    #[command(subcommand)]
    Dev(DevCmd),
}

#[derive(Debug, Parser)]
pub enum IsoCmd {
    /// Embed an Ignition config in an ISO image
    // deprecated
    #[command(hide = true)]
    Embed(IsoEmbedConfig),
    /// Show the embedded Ignition config from an ISO image
    // deprecated
    #[command(hide = true)]
    Show(IsoShowConfig),
    /// Remove an existing embedded Ignition config from an ISO image
    // deprecated
    #[command(hide = true)]
    Remove(IsoRemoveConfig),
    /// Customize a CoreOS live ISO image
    Customize(IsoCustomizeConfig),
    /// Embed an Ignition config in a CoreOS live ISO image
    #[command(subcommand)]
    Ignition(IsoIgnitionCmd),
    /// Embed network settings in a CoreOS live ISO image
    #[command(subcommand)]
    Network(IsoNetworkCmd),
    /// Modify kernel args in a CoreOS live ISO image
    #[command(subcommand)]
    Kargs(IsoKargsCmd),
    /// Commands to extract files from a CoreOS live ISO image
    #[command(subcommand)]
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
    #[command(subcommand)]
    Ignition(PxeIgnitionCmd),
    /// Commands to manage live PXE network settings
    #[command(subcommand)]
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
#[command(hide = true)]
pub enum PackCmd {
    /// Create osmet file from CoreOS block device
    Osmet(PackOsmetConfig),
    /// Pack a minimal ISO into a CoreOS live ISO image
    MinimalIso(PackMinimalIsoConfig),
    /// Generate man pages for coreos-installer
    #[cfg(feature = "docgen")]
    Man(PackManConfig),
    /// Generate example config file for install subcommand
    #[cfg(feature = "docgen")]
    ExampleConfig(PackExampleConfigConfig),
}

#[derive(Debug, Parser)]
// users shouldn't be interacting with this command normally
#[command(hide = true)]
pub enum DevCmd {
    /// Commands to show metadata
    #[command(subcommand)]
    Show(DevShowCmd),
    /// Commands to extract data
    #[command(subcommand)]
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
    #[arg(short, long, value_name = "name", default_value = "stable")]
    pub stream: String,
    /// Target CPU architecture
    #[arg(short, long, value_name = "name", default_value_t)]
    pub architecture: DefaultedString<Architecture>,
    /// Fedora CoreOS platform name
    #[arg(short, long, value_name = "name", default_value = "metal")]
    pub platform: String,
    /// Image format
    #[arg(short, long, value_name = "name", default_value = "raw.xz")]
    pub format: String,
    /// Manually specify the image URL
    #[arg(short = 'u', long, value_name = "URL")]
    pub image_url: Option<Url>,
    /// Destination directory
    #[arg(short = 'C', long, value_name = "path", default_value = ".")]
    pub directory: String,
    /// Decompress image and don't save signature
    #[arg(short, long)]
    pub decompress: bool,
    /// Allow unsigned image
    #[arg(long)]
    pub insecure: bool,
    /// Base URL for Fedora CoreOS stream metadata
    #[arg(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
    /// Fetch retries, or "infinite"
    #[arg(long, value_name = "N", default_value_t)]
    pub fetch_retries: FetchRetries,
}

#[derive(Debug, Parser)]
pub struct ListStreamConfig {
    /// Fedora CoreOS stream
    #[arg(short, long, value_name = "name", default_value = "stable")]
    pub stream: String,
    /// Base URL for Fedora CoreOS stream metadata
    #[arg(long, value_name = "URL")]
    pub stream_base_url: Option<Url>,
}

#[derive(Debug, Parser)]
pub struct CommonCustomizeConfig {
    /// Ignition config fragment for dest sys
    ///
    /// Automatically run installer and merge the specified Ignition config
    /// into the config for the destination system.
    #[arg(long, value_name = "path")]
    pub dest_ignition: Vec<String>,
    /// Install destination device
    ///
    /// Automatically run installer, installing to the specified destination
    /// device that the user must provide.  The resulting boot media will
    /// overwrite the destination device without confirmation.
    #[arg(long, value_name = "path", required_unless_present = "installer_config")]
    pub dest_device: Option<String>,
    /// Kernel and bootloader console for dest
    ///
    /// Automatically run installer, configuring the specified kernel and
    /// bootloader console for the destination system.  The argument uses
    /// the same syntax as the parameter to the "console=" kernel argument.
    #[arg(long, value_name = "spec")]
    pub dest_console: Vec<Console>,
    /// Destination kernel argument to append
    ///
    /// Automatically run installer, adding the specified kernel argument
    /// for every boot of the destination system.
    #[arg(long, value_name = "arg")]
    pub dest_karg_append: Vec<String>,
    /// Destination kernel argument to delete
    ///
    /// Automatically run installer, deleting the specified kernel argument
    /// for every boot of the destination system.
    #[arg(long, value_name = "arg")]
    pub dest_karg_delete: Vec<String>,
    /// NetworkManager keyfile for live & dest
    ///
    /// Configure networking using the specified NetworkManager keyfile.
    /// Network settings will be applied in the live environment, including
    /// when Ignition is run.  If installer is enabled via additional options,
    /// network settings will also be applied in the destination system,
    /// including when Ignition is run.
    #[arg(long, value_name = "path")]
    pub network_keyfile: Vec<String>,
    /// Nmstate file for live & dest
    ///
    /// Configure networking using NetworkManager keyfiles generated from the
    /// specified Nmstate files. Network settings will be applied in the live
    /// environment, including when Ignition is run.  If installer is enabled
    /// via additional options, network settings will also be applied in the
    /// destination system, including when Ignition is run.
    #[arg(long, value_name = "path")]
    pub network_nmstate: Vec<String>,
    /// Ignition PEM CA bundle for live & dest
    ///
    /// Specify additional TLS certificate authorities to be trusted by
    /// Ignition, in PEM format.  Authorities will be trusted by Ignition
    /// in the live environment and, if installer is enabled via additional
    /// options, in the destination system.
    #[arg(long, value_name = "path")]
    pub ignition_ca: Vec<String>,
    /// Script to run before installation
    ///
    /// If installer is run at boot, run this script before installation.
    /// If the script fails, the live environment will stop at an emergency
    /// shell.
    #[arg(long, value_name = "path")]
    pub pre_install: Vec<String>,
    /// Script to run after installation
    ///
    /// If installer is run at boot, run this script after installation.
    /// If the script fails, the live environment will stop at an emergency
    /// shell.
    #[arg(long, value_name = "path")]
    pub post_install: Vec<String>,
    /// Installer config file
    ///
    /// Automatically run coreos-installer and apply the specified installer
    /// config file.  Config files are applied in the order that they are
    /// specified.
    #[arg(long, value_name = "path")]
    pub installer_config: Vec<String>,
    /// Ignition config fragment for live env
    ///
    /// Merge the specified Ignition config into the config for the live
    /// environment.
    #[arg(long, value_name = "path")]
    pub live_ignition: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct IsoCustomizeConfig {
    // Customizations
    #[command(flatten)]
    pub common: CommonCustomizeConfig,
    /// Live kernel argument to append
    ///
    /// Kernel argument to append to boots of the live environment.
    #[arg(long, value_name = "arg")]
    pub live_karg_append: Vec<String>,
    /// Live kernel argument to delete
    ///
    /// Kernel argument to delete from boots of the live environment.
    #[arg(long, value_name = "arg")]
    pub live_karg_delete: Vec<String>,
    /// Live kernel argument to replace
    ///
    /// Kernel argument to replace for boots of the live environment, in the
    /// form key=old=new.  For a default argument "a=b", specifying
    /// "--live-karg-replace a=b=c" will produce the argument "a=c".
    #[arg(long, value_name = "k=o=n")]
    pub live_karg_replace: Vec<String>,

    // I/O configuration
    /// Overwrite existing customizations
    #[arg(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoEmbedConfig {
    /// Ignition config to embed [default: stdin]
    #[arg(short, long, value_name = "path")]
    pub config: Option<String>,
    /// Overwrite an existing embedded Ignition config
    #[arg(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoShowConfig {
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoRemoveConfig {
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoIgnitionEmbedConfig {
    /// Overwrite an existing Ignition config
    #[arg(short, long)]
    pub force: bool,
    /// Ignition config to embed [default: stdin]
    #[arg(short, long, value_name = "path")]
    pub ignition_file: Option<String>,
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoIgnitionShowConfig {
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoIgnitionRemoveConfig {
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoNetworkEmbedConfig {
    /// NetworkManager keyfile to embed
    // Required option. :-(  In future we might support other configuration
    // sources.
    #[arg(short, long, required = true, value_name = "path")]
    pub keyfile: Vec<String>,
    /// Overwrite existing network settings
    #[arg(short, long)]
    pub force: bool,
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoNetworkExtractConfig {
    /// Extract to directory instead of stdout
    #[arg(short = 'C', long, value_name = "path")]
    pub directory: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoNetworkRemoveConfig {
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoKargsModifyConfig {
    /// Kernel argument to append
    #[arg(short, long, value_name = "KARG")]
    pub append: Vec<String>,
    /// Kernel argument to delete
    #[arg(short, long, value_name = "KARG")]
    pub delete: Vec<String>,
    /// Kernel argument to replace
    #[arg(short, long, value_name = "KARG=OLDVAL=NEWVAL")]
    pub replace: Vec<String>,
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoKargsResetConfig {
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoKargsShowConfig {
    /// Show default kernel args
    #[arg(short, long)]
    pub default: bool,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct DevShowIsoConfig {
    /// Show Ignition embed area parameters
    #[arg(long, conflicts_with = "kargs")]
    pub ignition: bool,
    /// Show kargs embed area parameters
    #[arg(long, conflicts_with = "ignition")]
    pub kargs: bool,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct IsoExtractPxeConfig {
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
    /// Output directory
    #[arg(short, long, value_name = "PATH", default_value = ".")]
    pub output_dir: String,
}

#[derive(Debug, Parser)]
pub struct IsoExtractMinimalIsoConfig {
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
    /// Extract rootfs image as well
    #[arg(long, value_name = "PATH")]
    pub output_rootfs: Option<String>,
    /// Minimal ISO output file
    #[arg(value_name = "OUTPUT_ISO", default_value = "-")]
    pub output: String,
    /// Inject rootfs URL karg into minimal ISO
    #[arg(long, value_name = "URL")]
    pub rootfs_url: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PackMinimalIsoConfig {
    /// ISO image
    #[arg(value_name = "FULL_ISO")]
    pub full: String,
    /// Minimal ISO image
    #[arg(value_name = "MINIMAL_ISO")]
    pub minimal: String,
    /// Delete minimal ISO after packing
    #[arg(long)]
    pub consume: bool,
}

#[derive(Debug, Parser)]
pub struct IsoResetConfig {
    /// Write ISO to a new output file
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
    /// ISO image
    #[arg(value_name = "ISO")]
    pub input: String,
}

#[derive(Debug, Parser)]
// default usage line lists all mandatory options and so exceeds 80 characters
#[command(override_usage = "coreos-installer pack osmet [OPTIONS]")]
pub struct PackOsmetConfig {
    /// Path to osmet file to write
    // could output to stdout if missing?
    #[arg(long, required = true, value_name = "FILE")]
    pub output: String,
    /// Expected SHA256 of block device
    // XXX: rebase on top of
    // https://github.com/coreos/coreos-installer/pull/178 and use the same
    // type-digest format
    #[arg(long, required = true, value_name = "SHA256")]
    pub checksum: String,
    /// Description of OS
    #[arg(long, required = true, value_name = "TEXT")]
    pub description: String,
    /// Use worse compression, for development builds
    #[arg(long)]
    pub fast: bool,
    /// Source device
    #[arg(value_name = "DEV")]
    pub device: String,
}

#[derive(Debug, Parser)]
pub struct DevExtractOsmetConfig {
    /// osmet file
    #[arg(long, required = true, value_name = "PATH")]
    pub osmet: String,
    /// OSTree repo
    #[arg(value_name = "PATH")]
    pub repo: String,
    /// Destination device
    #[arg(value_name = "DEV")]
    pub device: String,
}

#[derive(Debug, Parser)]
pub struct DevShowFiemapConfig {
    /// File to map
    #[arg(value_name = "PATH")]
    pub file: String,
}

#[derive(Debug, Parser)]
pub struct PxeCustomizeConfig {
    // Customizations
    #[command(flatten)]
    pub common: CommonCustomizeConfig,

    // I/O configuration
    /// Output file
    #[arg(short, long, value_name = "path")]
    pub output: String,
    /// CoreOS live initramfs image
    #[arg(value_name = "path")]
    pub input: String,
}

#[derive(Debug, Parser)]
pub struct PxeIgnitionWrapConfig {
    /// Ignition config to wrap [default: stdin]
    #[arg(short, long, value_name = "path")]
    pub ignition_file: Option<String>,
    /// Write to a file instead of stdout
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PxeIgnitionUnwrapConfig {
    /// initrd image [default: stdin]
    #[arg(value_name = "initrd")]
    pub input: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PxeNetworkWrapConfig {
    /// NetworkManager keyfile to embed
    // Required option. :-(  In future we might support other configuration
    // sources.
    #[arg(short, long, required = true, value_name = "path")]
    pub keyfile: Vec<String>,
    /// Write to a file instead of stdout
    #[arg(short, long, value_name = "path")]
    pub output: Option<String>,
}

#[derive(Debug, Parser)]
pub struct PxeNetworkUnwrapConfig {
    /// Extract to directory instead of stdout
    #[arg(short = 'C', long, value_name = "path")]
    pub directory: Option<String>,
    /// initrd image [default: stdin]
    #[arg(value_name = "initrd")]
    pub input: Option<String>,
}

#[derive(Debug, Parser)]
pub struct DevShowInitrdConfig {
    /// initrd image ("-" for stdin)
    #[arg(value_name = "initrd")]
    pub input: String,
    /// Files or globs to list
    #[arg(value_name = "glob")]
    pub filter: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct DevExtractInitrdConfig {
    /// Output directory
    #[arg(short = 'C', long, value_name = "path", default_value = ".")]
    pub directory: String,
    /// List extracted contents
    #[arg(short, long)]
    pub verbose: bool,
    /// initrd image ("-" for stdin)
    #[arg(value_name = "initrd")]
    pub input: String,
    /// Files or globs to list
    #[arg(value_name = "glob")]
    pub filter: Vec<String>,
}

#[cfg(feature = "docgen")]
#[derive(Debug, Parser)]
pub struct PackManConfig {
    /// Output directory
    #[arg(short = 'C', long, value_name = "path", default_value = ".")]
    pub directory: String,
}

#[cfg(feature = "docgen")]
#[derive(Debug, Parser)]
pub struct PackExampleConfigConfig {}

#[cfg(test)]
mod test {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_app() {
        Cmd::command().debug_assert()
    }
}
