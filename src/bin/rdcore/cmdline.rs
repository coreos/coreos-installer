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

use clap::{AppSettings, Parser};

// Args are listed in --help in the order declared in these structs/enums.

#[derive(Debug, Parser)]
#[clap(name = "rdcore", version)]
#[clap(global_setting(AppSettings::DeriveDisplayOrder))]
#[clap(args_conflicts_with_subcommands = true)]
#[clap(disable_help_subcommand = true)]
#[clap(help_expected = true)]
pub enum Cmd {
    /// Generate rootmap kargs and optionally inject into BLS configs
    Rootmap(RootmapConfig),
    /// Generate bootmap kargs and binds bootfs to rootfs and GRUB
    BindBoot(BindBootConfig),
    /// Modify kargs in BLS configs
    Kargs(KargsConfig),
    /// Copy data from stdin to stdout, checking piecewise hashes
    StreamHash(StreamHashConfig),
    /// Checks there is only one filesystem with given label
    VerifyUniqueFsLabel(VerifyUniqueFsLabelConfig),
}

#[derive(Debug, Parser)]
pub struct RootmapConfig {
    // we allow mounting /boot ourselves (--boot-device) or letting our
    // caller do the mount and point us to it (--boot-mount); lots of
    // backstory on /boot mounting in the initrd, so leave some flexibility
    // for changing implementation details on the OS side without having to
    // respin rdcore
    /// Boot device containing BLS entries to modify
    #[clap(long, value_name = "DEVPATH", conflicts_with = "boot-mount")]
    pub boot_device: Option<String>,
    /// Boot mount containing BLS entries to modify
    #[clap(long, value_name = "BOOT_MOUNT", conflicts_with = "boot-device")]
    pub boot_mount: Option<String>,
    /// Path to rootfs mount
    #[clap(value_name = "ROOT_MOUNT")]
    pub root_mount: String,
}

#[derive(Debug, Parser)]
pub struct BindBootConfig {
    /// Path to rootfs mount
    #[clap(value_name = "ROOT_MOUNT")]
    pub root_mount: String,
    /// Path to bootfs mount
    #[clap(value_name = "BOOT_MOUNT")]
    pub boot_mount: String,
}

#[derive(Debug, Parser)]
pub struct KargsConfig {
    // see comment block in rootmap command above
    /// Boot device containing BLS entries to modify
    #[clap(long, value_name = "DEVPATH")]
    #[clap(conflicts_with = "boot-mount", conflicts_with = "current")]
    pub boot_device: Option<String>,
    /// Boot mount containing BLS entries to modify
    #[clap(long, value_name = "BOOT_MOUNT")]
    #[clap(conflicts_with = "boot-device", conflicts_with = "current")]
    pub boot_mount: Option<String>,
    /// Dry run using kargs from this boot
    #[clap(long)]
    #[clap(conflicts_with = "boot-device", conflicts_with = "boot-mount")]
    pub current: bool,
    /// Modify this option string instead of fetching from BLS entry
    // this is purely for dev testing
    #[clap(long, value_name = "OPTIONS", hide = true)]
    pub override_options: Option<String>,
    /// File to create if BLS entry was modified
    #[clap(long, value_name = "PATH")]
    pub create_if_changed: Option<String>,
    /// Append kernel arg
    #[clap(long, value_name = "ARG")]
    pub append: Vec<String>,
    /// Append kernel arg if missing
    #[clap(long, value_name = "ARG")]
    #[clap(alias = "should-exist")]
    pub append_if_missing: Vec<String>,
    /// Delete kernel arg
    #[clap(long, value_name = "ARG")]
    #[clap(alias = "should-not-exist")]
    pub delete: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct StreamHashConfig {
    /// Path to the piecewise hash file
    #[clap(value_name = "hash-file")]
    pub hash_file: String,
}

#[derive(Debug, Parser)]
pub struct VerifyUniqueFsLabelConfig {
    /// Filesystem's label
    #[clap(value_name = "LABEL")]
    pub label: String,

    /// Force rereading of partition table
    #[clap(long)]
    pub rereadpt: bool,
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
