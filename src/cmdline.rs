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

use clap::{crate_version, App, AppSettings, Arg, ArgMatches, SubCommand};
use error_chain::bail;
use reqwest::Url;

use crate::errors::*;
use crate::source::*;

pub enum Config {
    Install(InstallConfig),
}

pub struct InstallConfig {
    pub device: String,
    pub location: Box<dyn ImageLocation>,
    pub ignition: Option<String>,
    pub platform: Option<String>,
    pub firstboot_kargs: Option<String>,
    pub insecure: bool,
}

/// Parse command-line arguments.
pub fn parse_args() -> Result<Config> {
    let uname = nix::sys::utsname::uname();
    // Args are listed in --help in the order declared here.  Please keep
    // the entire help text to 80 columns.
    let app_matches = App::new("coreos-installer")
        .version(crate_version!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .global_setting(AppSettings::ArgsNegateSubcommands)
        .global_setting(AppSettings::DeriveDisplayOrder)
        .global_setting(AppSettings::DisableHelpSubcommand)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .global_setting(AppSettings::VersionlessSubcommands)
        .subcommand(
            SubCommand::with_name("install")
                .about("Install Fedora CoreOS or RHEL CoreOS")
                // ways to specify the image source
                .arg(
                    Arg::with_name("stream")
                        .short("s")
                        .long("stream")
                        .value_name("name")
                        .help("Fedora CoreOS stream")
                        .default_value("stable")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("image-url")
                        .short("u")
                        .long("image-url")
                        .value_name("URL")
                        .help("Manually specify the image URL")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("image-file")
                        .short("f")
                        .long("image-file")
                        .value_name("path")
                        .help("Manually specify a local image file")
                        .takes_value(true),
                )
                // postprocessing options
                .arg(
                    Arg::with_name("ignition-path")
                        .short("i")
                        .long("ignition")
                        .value_name("path")
                        .help("Embed an Ignition config")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("platform")
                        .short("p")
                        .long("platform")
                        .value_name("name")
                        .help("Override the Ignition platform ID")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("firstboot-kargs")
                        .long("firstboot-args")
                        .value_name("args")
                        .help("Additional kernel args for the first boot")
                        .takes_value(true),
                )
                // obscure options without short names
                .arg(
                    Arg::with_name("insecure")
                        .long("insecure")
                        .help("Skip signature verification"),
                )
                .arg(
                    Arg::with_name("stream-base-url")
                        .long("stream-base-url")
                        .value_name("URL")
                        .help("Base URL for Fedora CoreOS stream metadata")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("architecture")
                        .long("architecture")
                        .value_name("name")
                        .help("Target CPU architecture")
                        .default_value(uname.machine())
                        .takes_value(true),
                )
                // positional args
                .arg(
                    Arg::with_name("device")
                        .help("Destination device")
                        .required(true)
                        .takes_value(true),
                ),
        )
        .get_matches();

    match app_matches.subcommand() {
        ("install", Some(matches)) => parse_install(&matches),
        _ => bail!("unrecognized subcommand"),
    }
}

fn parse_install(matches: &ArgMatches) -> Result<Config> {
    // Build image location.  Ideally we'd use conflicts_with (and an
    // ArgGroup for streams), but that doesn't play well with default
    // arguments, so we manually prioritize modes.
    let location: Box<dyn ImageLocation> = if matches.is_present("image-file") {
        Box::new(FileLocation::new(
            matches.value_of("image-file").expect("image-file missing"),
        ))
    } else if matches.is_present("image-url") {
        let image_url = Url::parse(matches.value_of("image-url").expect("image-url missing"))
            .chain_err(|| "parsing image URL")?;
        Box::new(UrlLocation::new(&image_url))
    } else {
        let base_url = if matches.is_present("stream-base-url") {
            Some(
                Url::parse(
                    matches
                        .value_of("stream-base-url")
                        .expect("stream-base-url missing"),
                )
                .chain_err(|| "parsing stream base URL")?,
            )
        } else {
            None
        };
        Box::new(StreamLocation::new(
            matches.value_of("stream").expect("stream missing"),
            matches
                .value_of("architecture")
                .expect("architecture missing"),
            base_url.as_ref(),
        )?)
    };
    // and report it to the user
    println!("{}", location);

    // build configuration
    Ok(Config::Install(InstallConfig {
        device: matches
            .value_of("device")
            .map(String::from)
            .expect("device missing"),
        location,
        ignition: matches.value_of("ignition-path").map(String::from),
        platform: matches.value_of("platform").map(String::from),
        firstboot_kargs: matches.value_of("firstboot-kargs").map(String::from),
        insecure: matches.is_present("insecure"),
    }))
}
