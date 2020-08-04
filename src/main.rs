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

use libcoreinst::{cmdline, download, errors, install, live, osmet, source};

use cmdline::Config;
use error_chain::quick_main;
use errors::{Result, ResultExt};

quick_main!(run);

fn run() -> Result<()> {
    let config = cmdline::parse_args().chain_err(|| "parsing arguments")?;

    match config {
        Config::Download(c) => download::download(&c),
        Config::ListStream(c) => source::list_stream(&c),
        Config::Install(c) => install::install(&c),
        Config::IsoEmbed(c) => live::iso_embed(&c),
        Config::IsoShow(c) => live::iso_show(&c),
        Config::IsoRemove(c) => live::iso_remove(&c),
        Config::IsoIgnitionEmbed(c) => live::iso_ignition_embed(&c),
        Config::IsoIgnitionShow(c) => live::iso_ignition_show(&c),
        Config::IsoIgnitionRemove(c) => live::iso_ignition_remove(&c),
        Config::IsoKargsModify(c) => live::iso_kargs_modify(&c),
        Config::IsoKargsReset(c) => live::iso_kargs_reset(&c),
        Config::IsoKargsShow(c) => live::iso_kargs_show(&c),
        Config::OsmetFiemap(c) => osmet::osmet_fiemap(&c),
        Config::OsmetPack(c) => osmet::osmet_pack(&c),
        Config::OsmetUnpack(c) => osmet::osmet_unpack(&c),
        Config::PxeIgnitionWrap(c) => live::pxe_ignition_wrap(&c),
        Config::PxeIgnitionUnwrap(c) => live::pxe_ignition_unwrap(&c),
    }
}
