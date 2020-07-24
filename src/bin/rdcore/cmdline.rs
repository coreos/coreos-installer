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
    StreamHash(StreamHashConfig),
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
        ("stream-hash", Some(matches)) => parse_stream_hash(&matches),
        _ => bail!("unrecognized subcommand"),
    }
}

fn parse_stream_hash(matches: &ArgMatches) -> Result<Config> {
    Ok(Config::StreamHash(StreamHashConfig {
        hash_file: matches
            .value_of("hash-file")
            .map(String::from)
            .expect("hash file missing"),
    }))
}
