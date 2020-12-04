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

use libcoreinst::errors::*;
use libcoreinst::install::*;

use crate::cmdline::*;
use crate::rootmap::get_boot_mount_from_cmdline_args;

pub fn kargs(config: &KargsConfig) -> Result<()> {
    // the unwrap() here is safe because we checked in cmdline that one of them must be provided
    let mount = get_boot_mount_from_cmdline_args(&config.boot_mount, &config.boot_device)?.unwrap();
    visit_bls_entry_options(mount.mountpoint(), |orig_options: &str| {
        let new_options = bls_entry_delete_and_append_kargs(
            orig_options,
            config.delete_kargs.as_ref(),
            config.append_kargs.as_ref(),
        )?;

        // we always print the final kargs
        if let Some(ref options) = new_options {
            println!("{}", options);
        } else {
            println!("{}", orig_options);
        }

        Ok(new_options)
    })
    .chain_err(|| "visiting BLS options")?;

    Ok(())
}
