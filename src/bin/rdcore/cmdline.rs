// Copyright 2020 CoreOS, Inc.
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

// For consistency, have all parse_*() functions return Result.
#![allow(clippy::unnecessary_wraps)]

use anyhow::{bail, Result};
use structopt::clap::AppSettings;
use structopt::StructOpt;

// Args are listed in --help in the order declared in these structs/enums.

#[derive(Debug, StructOpt)]
#[structopt(name = "rdcore")]
#[structopt(global_setting(AppSettings::ArgsNegateSubcommands))]
#[structopt(global_setting(AppSettings::DeriveDisplayOrder))]
#[structopt(global_setting(AppSettings::DisableHelpSubcommand))]
#[structopt(global_setting(AppSettings::UnifiedHelpMessage))]
#[structopt(global_setting(AppSettings::VersionlessSubcommands))]
pub enum Cmd {
    /// Generate rootmap kargs and optionally inject into BLS configs
    Rootmap(RootmapConfig),
    /// Modify kargs in BLS configs
    Kargs(KargsConfig),
    /// Copy data from stdin to stdout, checking piecewise hashes
    StreamHash(StreamHashConfig),
}

#[derive(Debug, StructOpt)]
pub struct RootmapConfig {
    // we allow mounting /boot ourselves (--boot-device) or letting our
    // caller do the mount and point us to it (--boot-mount); lots of
    // backstory on /boot mounting in the initrd, so leave some flexibility
    // for changing implementation details on the OS side without having to
    // respin rdcore
    /// Boot device containing BLS entries to modify
    #[structopt(long, value_name = "DEVPATH", conflicts_with = "boot-mount")]
    pub boot_device: Option<String>,
    /// Boot mount containing BLS entries to modify
    #[structopt(long, value_name = "BOOT_MOUNT", conflicts_with = "boot-device")]
    pub boot_mount: Option<String>,
    /// Path to rootfs mount
    #[structopt(value_name = "ROOT_MOUNT")]
    pub root_mount: String,
}

#[derive(Debug, StructOpt)]
pub struct KargsConfig {
    // see comment block in rootmap command above
    /// Boot device containing BLS entries to modify
    #[structopt(long, value_name = "DEVPATH")]
    #[structopt(conflicts_with = "boot-mount", conflicts_with = "current")]
    pub boot_device: Option<String>,
    /// Boot mount containing BLS entries to modify
    #[structopt(long, value_name = "BOOT_MOUNT")]
    #[structopt(conflicts_with = "boot-device", conflicts_with = "current")]
    pub boot_mount: Option<String>,
    /// Dry run using kargs from this boot
    #[structopt(long)]
    #[structopt(conflicts_with = "boot-device", conflicts_with = "boot-mount")]
    pub current: bool,
    /// Modify this option string instead of fetching from BLS entry
    // this is purely for dev testing
    #[structopt(long, value_name = "OPTIONS", hidden = true)]
    pub override_options: Option<String>,
    /// File to create if BLS entry was modified
    #[structopt(long, value_name = "PATH")]
    pub create_if_changed: Option<String>,
    /// Append kernel arg
    #[structopt(long, value_name = "ARG", number_of_values = 1)]
    pub append: Vec<String>,
    /// Append kernel arg if missing
    #[structopt(long, value_name = "ARG", number_of_values = 1)]
    #[structopt(alias = "should-exist")]
    pub append_if_missing: Vec<String>,
    /// Delete kernel arg
    #[structopt(long, value_name = "ARG", number_of_values = 1)]
    #[structopt(alias = "should-not-exist")]
    pub delete: Vec<String>,
}

#[derive(Debug, StructOpt)]
pub struct StreamHashConfig {
    /// Path to the piecewise hash file
    #[structopt(value_name = "hash-file")]
    pub hash_file: String,
}

/// Parse command-line arguments.
pub fn parse_args() -> Result<Cmd> {
    let config = Cmd::from_args();
    if let Cmd::Kargs(ref config) = config {
        check_kargs(config)?;
    }
    Ok(config)
}

fn check_kargs(config: &KargsConfig) -> Result<()> {
    // we could enforce these via clap's ArgGroup, but I don't like how the --help text looks
    if !(config.boot_device.is_some()
        || config.boot_mount.is_some()
        || config.current
        || config.override_options.is_some())
    {
        // --override-options is undocumented on purpose
        bail!("one of --boot-device, --boot-mount, or --current required");
    }
    Ok(())
}
