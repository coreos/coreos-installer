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
use std::process::Command;

/// Runs the provided command. The first macro argument is the executable, and following arguments
/// are passed to the command. Returns a Result<()> describing whether the command failed. Errors
/// are adequately prefixed with the full command.
#[macro_export]
macro_rules! runcmd {
    ($cmd:expr) => (runcmd!($cmd,));
    ($cmd:expr, $($args:expr),*) => {{
        let mut cmd = Command::new($cmd);
        $( cmd.arg($args); )*
        let status = cmd.status().with_context(|| format!("running {:#?}", cmd))?;
        if !status.success() {
            Result::Err(anyhow!("{:#?} failed with {}", cmd, status))
        } else {
            Result::Ok(())
        }
    }}
}

/// Runs the provided command, captures its stdout, and swallows its stderr except on failure.
/// The first macro argument is the executable, and following arguments are passed to the command.
/// Returns a Result<String> describing whether the command failed, and if not, its standard
/// output. Output is assumed to be UTF-8. Errors are adequately prefixed with the full command.
#[macro_export]
macro_rules! runcmd_output {
    ($cmd:expr) => (runcmd_output!($cmd,));
    ($cmd:expr, $($args:expr),*) => {{
        let mut cmd = Command::new($cmd);
        $( cmd.arg($args); )*
        // NB: cmd_output already prefixes with cmd in all error paths
        cmd_output(&mut cmd)
    }}
}

/// Runs the provided Command object, captures its stdout, and swallows its stderr except on
/// failure. Returns a Result<String> describing whether the command failed, and if not, its
/// standard output. Output is assumed to be UTF-8. Errors are adequately prefixed with the full
/// command.
pub fn cmd_output(cmd: &mut Command) -> Result<String> {
    let result = cmd
        .output()
        .with_context(|| format!("running {:#?}", cmd))?;
    if !result.status.success() {
        eprint!("{}", String::from_utf8_lossy(&result.stderr));
        bail!("{:#?} failed with {}", cmd, result.status);
    }
    String::from_utf8(result.stdout)
        .with_context(|| format!("decoding as UTF-8 output of `{:#?}`", cmd))
}
