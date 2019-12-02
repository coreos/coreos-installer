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

mod blockdev;
mod cmdline;
mod download;
mod errors;
mod install;
mod iso;
mod source;
mod verify;

use crate::cmdline::Config;
use crate::errors::{Result, ResultExt};
use error_chain::quick_main;

quick_main!(run);

fn run() -> Result<()> {
    let config = cmdline::parse_args().chain_err(|| "parsing arguments")?;

    match config {
        Config::Download(c) => download::download(&c),
        Config::ListStream(c) => source::list_stream(&c),
        Config::Install(c) => install::install(&c),
        Config::IsoEmbed(c) => iso::iso_embed(&c),
        Config::IsoShow(c) => iso::iso_show(&c),
        Config::IsoRemove(c) => iso::iso_remove(&c),
    }
}
