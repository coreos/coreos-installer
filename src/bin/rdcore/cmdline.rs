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

use clap::{crate_version, App, AppSettings, Arg, ArgMatches, SubCommand};
use error_chain::bail;

use libcoreinst::errors::*;

pub enum Config {
    Kargs(KargsConfig),
    RootMap(RootMapConfig),
    StreamHash(StreamHashConfig),
}

pub struct KargsConfig {
    pub boot_device: Option<String>,
    pub boot_mount: Option<String>,
    pub append_kargs: Vec<String>,
    pub delete_kargs: Vec<String>,
}

pub struct RootMapConfig {
    pub boot_device: Option<String>,
    pub boot_mount: Option<String>,
    pub root_mount: String,
}

pub struct StreamHashConfig {
    pub hash_file: String,
}

/// Parse command-line arguments.
pub fn parse_args() -> Result<Config> {
    // Args are listed in --help in the order declared here.  Please keep
    // the entire help text to 80 columns.
    let app_matches = App::new("rdcore")
        .version(crate_version!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .global_setting(AppSettings::ArgsNegateSubcommands)
        .global_setting(AppSettings::DeriveDisplayOrder)
        .global_setting(AppSettings::DisableHelpSubcommand)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .global_setting(AppSettings::VersionlessSubcommands)
        .subcommand(
            SubCommand::with_name("rootmap")
                .about("Generate rootmap kargs and optionally inject into BLS configs")
                .arg(
                    Arg::with_name("root-mount")
                        .help("Path to rootfs mount")
                        .required(true)
                        .value_name("ROOT_MOUNT")
                        .takes_value(true),
                )
                // we allow mounting /boot ourselves (--boot-device) or letting our caller do the
                // mount and point us to it (--boot-mount); lots of backstory on /boot mounting in
                // the initrd, so leave some flexibility for changing implementation details on the
                // OS side without having to respin rdcore
                .arg(
                    Arg::with_name("boot-device")
                        .long("boot-device")
                        .help("Boot device containing BLS entries to modify")
                        .conflicts_with("boot-mount")
                        .value_name("DEVPATH")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("boot-mount")
                        .long("boot-mount")
                        .help("Boot mount containing BLS entries to modify")
                        .conflicts_with("boot-device")
                        .value_name("BOOT_MOUNT")
                        .takes_value(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("kargs")
                .about("Modify kargs in BLS configs")
                // see comment block in rootmap command above
                .arg(
                    Arg::with_name("boot-device")
                        .long("boot-device")
                        .help("Boot device containing BLS entries to modify")
                        .conflicts_with("boot-mount")
                        .value_name("DEVPATH")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("boot-mount")
                        .long("boot-mount")
                        .help("Boot mount containing BLS entries to modify")
                        .conflicts_with("boot-device")
                        .value_name("BOOT_MOUNT")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("append")
                        .long("append")
                        .value_name("ARG")
                        .help("Append kernel arg")
                        .takes_value(true)
                        .number_of_values(1)
                        .multiple(true),
                )
                .arg(
                    Arg::with_name("delete")
                        .long("delete")
                        .value_name("ARG")
                        .help("Delete kernel arg")
                        .takes_value(true)
                        .number_of_values(1)
                        .multiple(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("stream-hash")
                .about("Copy data from stdin to stdout, checking piecewise hashes")
                .arg(
                    Arg::with_name("hash-file")
                        .value_name("hash-file")
                        .help("Path to the piecewise hash file")
                        .required(true)
                        .takes_value(true),
                ),
        )
        .get_matches();

    match app_matches.subcommand() {
        ("kargs", Some(matches)) => parse_kargs(&matches),
        ("rootmap", Some(matches)) => parse_rootmap(&matches),
        ("stream-hash", Some(matches)) => parse_stream_hash(&matches),
        _ => bail!("unrecognized subcommand"),
    }
}

fn parse_kargs(matches: &ArgMatches) -> Result<Config> {
    // we could enforce these via clap's ArgGroup, but I don't like how the --help text looks
    if !(matches.is_present("boot-device") || matches.is_present("boot-mount")) {
        bail!("at least one of --boot-device or --boot-mount required");
    }
    Ok(Config::Kargs(KargsConfig {
        boot_device: matches.value_of("boot-device").map(String::from),
        boot_mount: matches.value_of("boot-mount").map(String::from),
        append_kargs: matches
            .values_of("append")
            .map(|v| v.map(String::from).collect())
            .unwrap_or_else(Vec::new),
        delete_kargs: matches
            .values_of("delete")
            .map(|v| v.map(String::from).collect())
            .unwrap_or_else(Vec::new),
    }))
}

fn parse_rootmap(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::RootMap(RootMapConfig {
        boot_device: matches.value_of("boot-device").map(String::from),
        boot_mount: matches.value_of("boot-mount").map(String::from),
        root_mount: matches
            .value_of("root-mount")
            .map(String::from)
            .expect("rootfs mount missing"),
    }))
}

fn parse_stream_hash(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::StreamHash(StreamHashConfig {
        hash_file: matches
            .value_of("hash-file")
            .map(String::from)
            .expect("hash file missing"),
    }))
}
