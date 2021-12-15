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

use anyhow::Result;
use structopt::StructOpt;

use libcoreinst::{cmdline, download, install, live, osmet, source};

use cmdline::*;

fn main() -> Result<()> {
    match Cmd::from_args() {
        Cmd::Download(c) => download::download(c),
        Cmd::Install(c) => install::install(c),
        Cmd::ListStream(c) => source::list_stream(c),
        Cmd::Iso(c) => match c {
            IsoCmd::Customize(c) => live::iso_customize(c),
            IsoCmd::Embed(c) => live::iso_embed(c),
            IsoCmd::Show(c) => live::iso_show(c),
            IsoCmd::Remove(c) => live::iso_remove(c),
            IsoCmd::Ignition(c) => match c {
                IsoIgnitionCmd::Embed(c) => live::iso_ignition_embed(c),
                IsoIgnitionCmd::Show(c) => live::iso_ignition_show(c),
                IsoIgnitionCmd::Remove(c) => live::iso_ignition_remove(c),
            },
            IsoCmd::Network(c) => match c {
                IsoNetworkCmd::Embed(c) => live::iso_network_embed(c),
                IsoNetworkCmd::Extract(c) => live::iso_network_extract(c),
                IsoNetworkCmd::Remove(c) => live::iso_network_remove(c),
            },
            IsoCmd::Kargs(c) => match c {
                IsoKargsCmd::Modify(c) => live::iso_kargs_modify(c),
                IsoKargsCmd::Reset(c) => live::iso_kargs_reset(c),
                IsoKargsCmd::Show(c) => live::iso_kargs_show(c),
            },
            IsoCmd::Inspect(c) => live::iso_inspect(c),
            IsoCmd::Extract(c) => match c {
                IsoExtractCmd::Pxe(c) => live::iso_extract_pxe(c),
                IsoExtractCmd::MinimalIso(c) => live::iso_extract_minimal_iso(c),
                IsoExtractCmd::PackMinimalIso(c) => live::iso_pack_minimal_iso(c),
            },
            IsoCmd::Reset(c) => live::iso_reset(c),
        },
        Cmd::Osmet(c) => match c {
            OsmetCmd::Fiemap(c) => osmet::osmet_fiemap(c),
            OsmetCmd::Pack(c) => osmet::osmet_pack(c),
            OsmetCmd::Unpack(c) => osmet::osmet_unpack(c),
        },
        Cmd::Pxe(c) => match c {
            PxeCmd::Customize(c) => live::pxe_customize(c),
            PxeCmd::Ignition(c) => match c {
                PxeIgnitionCmd::Wrap(c) => live::pxe_ignition_wrap(c),
                PxeIgnitionCmd::Unwrap(c) => live::pxe_ignition_unwrap(c),
            },
            PxeCmd::Network(c) => match c {
                PxeNetworkCmd::Wrap(c) => live::pxe_network_wrap(c),
                PxeNetworkCmd::Unwrap(c) => live::pxe_network_unwrap(c),
            },
        },
    }
}
