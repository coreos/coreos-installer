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

//! Support for generating man pages.

use anyhow::{Context, Result};
use clap::{crate_version, Command, CommandFactory};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::io::BUFFER_SIZE;

use super::{Cmd, PackManConfig};

pub fn pack_man(config: PackManConfig) -> Result<()> {
    pack_one(&config, Cmd::command())
}

fn pack_one(config: &PackManConfig, cmd: Command) -> Result<()> {
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
        .with_context(|| format!("rendering {}.8", name))?;
    buf.flush().context("flushing man page")?;
    drop(buf);

    for subcmd in cmd.get_subcommands().filter(|c| !c.is_hide_set()) {
        let subname = format!("{}-{}", name, subcmd.get_name());
        pack_one(
            config,
            subcmd.clone().name(subname).version(crate_version!()),
        )?;
    }
    Ok(())
}
