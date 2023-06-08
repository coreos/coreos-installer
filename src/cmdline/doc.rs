// Copyright 2022 Red Hat
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

//! Support for generating docs.

use anyhow::{bail, Context, Result};
use clap::{crate_version, ArgAction, Command, CommandFactory};
use serde::{de, forward_to_deserialize_any, Deserialize};
use std::collections::{hash_map::RandomState, HashMap};
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::io::BUFFER_SIZE;

use super::{Cmd, InstallConfig, PackExampleConfigConfig, PackManConfig};

pub fn pack_man(config: PackManConfig) -> Result<()> {
    pack_one_man(&config, Cmd::command())
}

fn pack_one_man(config: &PackManConfig, cmd: Command) -> Result<()> {
    let name = cmd.get_name();
    let path = Path::new(&config.directory).join(format!("{}.8", name));
    println!("Generating {}...", path.display());

    let out = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut buf = BufWriter::with_capacity(BUFFER_SIZE, out);
    clap_mangen::Man::new(cmd.clone())
        .title("coreos-installer")
        .section("8")
        .source(format!("coreos-installer {}", crate_version!()))
        .render(&mut buf)
        .with_context(|| format!("rendering {name}.8"))?;
    buf.flush().context("flushing man page")?;
    drop(buf);

    for subcmd in cmd.get_subcommands().filter(|c| !c.is_hide_set()) {
        let subname = format!("{name}-{}", subcmd.get_name());
        pack_one_man(
            config,
            subcmd.clone().name(subname).version(crate_version!()),
        )?;
    }
    Ok(())
}

pub fn pack_example_config(_config: PackExampleConfigConfig) -> Result<()> {
    // first get the field list from serde's perspective, so we omit any
    // fields hidden from it
    let mut fields = Vec::new();
    // we usually skip empty fields on serialize, so we have to do this
    // via the deserializer instead
    InstallConfig::deserialize(&mut FieldLister {
        fields: &mut fields,
    })
    .unwrap_err();

    // now use that list to query clap arguments and format output
    let mut out = String::new();
    let cmd = InstallConfig::command();
    // argument name -> clap::Arg
    let arg_map: HashMap<_, _, RandomState> =
        HashMap::from_iter(cmd.get_arguments().map(|arg| (arg.get_id().as_str(), arg)));
    for field in &fields {
        if let Some(arg) = arg_map.get(&*field.replace('-', "_")) {
            // output help comment
            // we can't serialize through serde_yaml because it doesn't
            // support comments
            if let Some(help) = arg.get_help() {
                // strip StyledStr
                let help = format!("{}", help);
                let help = match field.as_ref() {
                    // clarify YAML syntax for "infinite"
                    "fetch-retries" => r#"Fetch retries, or string "infinite""#,
                    // don't reference -n short option
                    "network-dir" => "Source directory for copy-network",
                    // "arg" => "arguments"
                    "append-karg" => "Append default kernel arguments",
                    "delete-karg" => "Delete default kernel arguments",
                    _ => &help,
                };
                writeln!(out, "# {help}").unwrap();
            }

            // output "field: argument-description"
            let value_names = arg.get_value_names().map(|v| v[0].as_str());
            let desc = match arg.get_action() {
                ArgAction::Set => {
                    // option with argument
                    match field.as_ref() {
                        // positional arguments have different formatting
                        "dest-device" => "path",
                        _ => value_names.expect("missing value name"),
                    }
                    .into()
                }
                ArgAction::Append => {
                    // value array
                    let value_name = match field.as_ref() {
                        // more verbose than 80 columns will allow
                        "save-partlabel" => "glob",
                        "save-partindex" => "id-or-range",
                        _ => value_names.expect("missing value name"),
                    };
                    format!("[{0}, {0}]", value_name)
                }
                _ => {
                    // option flag
                    "true".into()
                }
            };
            writeln!(out, "{field}: {desc}").unwrap();
        } else {
            bail!("couldn't look up field {}", field);
        }
    }

    // since we hand-rolled YAML output, make sure it parses
    serde_yaml::from_str::<serde_yaml::Value>(&out).context("re-parsing output")?;

    print!("{}", out);
    Ok(())
}

struct FieldLister<'a> {
    fields: &'a mut Vec<String>,
}

impl<'de, 'a> de::Deserializer<'de> for &'a mut FieldLister<'a> {
    type Error = de::value::Error;

    fn deserialize_struct<V: de::Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.fields.extend(fields.iter().map(|v| v.to_string()));
        // we don't want to bother generating a default struct and throwing
        // it away, so fail
        Err(de::Error::custom("expected failure"))
    }

    // fill out the API contract
    fn deserialize_any<V: de::Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        unimplemented!()
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}
