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

use anyhow::{bail, Context, Result};
use std::fs::read_to_string;

use libcoreinst::io::*;
#[cfg(target_arch = "s390x")]
use libcoreinst::s390x;

use crate::cmdline::*;
use crate::rootmap::get_boot_mount_from_cmdline_args;

pub fn kargs(config: KargsConfig) -> Result<()> {
    // we could enforce these via clap's ArgGroup, but I don't like how the --help text looks
    if !(config.boot_device.is_some()
        || config.boot_mount.is_some()
        || config.current
        || config.override_options.is_some())
    {
        // --override-options is undocumented on purpose
        bail!("one of --boot-device, --boot-mount, or --current required");
    }

    if let Some(orig_options) = &config.override_options {
        modify_and_print(&config, orig_options.trim()).context("modifying options")?;
    } else if config.current {
        let orig_options =
            read_to_string("/proc/cmdline").context("reading kernel command line")?;
        modify_and_print(&config, orig_options.trim()).context("modifying options")?;
    } else {
        // the unwrap() here is safe because we checked in cmdline that one of them must be provided
        let mount =
            get_boot_mount_from_cmdline_args(&config.boot_mount, &config.boot_device)?.unwrap();
        let _changed = visit_bls_entry_options(mount.mountpoint(), |orig_options: &str| {
            modify_and_print(&config, orig_options)
        })
        .context("visiting BLS options")?;

        #[cfg(target_arch = "s390x")]
        if _changed {
            s390x::zipl(mount.mountpoint())?;
        }
    }

    Ok(())
}

fn modify_and_print(config: &KargsConfig, orig_options: &str) -> Result<Option<String>> {
    let new_options = KargsEditor::new()
        .delete(config.delete.as_slice())
        .append(config.append.as_slice())
        .append_if_missing(config.append_if_missing.as_slice())
        .maybe_apply_to(orig_options)?;

    // we always print the final kargs
    if let Some(options) = &new_options {
        println!("{}", options);
        if options != orig_options {
            if let Some(path) = &config.create_if_changed {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .open(path)
                    .with_context(|| format!("creating {}", path))?;
            }
        }
    } else {
        println!("{}", orig_options);
    }

    Ok(new_options)
}
