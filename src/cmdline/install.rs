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

//! Struct definition and support code for install subcommand.

use anyhow::{Context, Result};
use clap::Parser;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};
use std::default::Default;
use std::ffi::OsStr;
use std::fs::OpenOptions;

use crate::io::IgnitionHash;

use super::serializer;
use super::types::*;
use super::Cmd;

// Args are listed in --help in the order declared in these structs/enums.
// Please keep the entire help text to 80 columns.

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

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;
    use std::num::NonZeroU32;
    use std::str::FromStr;
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
