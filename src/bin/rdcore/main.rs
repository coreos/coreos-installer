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

mod cmdline;
mod kargs;
mod rootmap;
mod stream_hash;
mod unique_fs;

use anyhow::Result;
use clap::Parser;

use crate::cmdline::*;

fn main() -> Result<()> {
    match Cmd::parse() {
        Cmd::Kargs(c) => kargs::kargs(c),
        Cmd::Rootmap(c) => rootmap::rootmap(c),
        Cmd::BindBoot(c) => rootmap::bind_boot(c),
        Cmd::StreamHash(c) => stream_hash::stream_hash(c),
        Cmd::VerifyUniqueFsLabel(c) => unique_fs::verify_unique_fs(c),
    }
}
