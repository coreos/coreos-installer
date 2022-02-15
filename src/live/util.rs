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

use anyhow::{bail, Context, Result};
use nix::unistd::isatty;
use std::fs::{write, File, OpenOptions};
use std::io::{self, copy, BufWriter, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

use crate::io::*;
use crate::iso9660::{self, IsoFs};

use super::embed::IsoConfig;

// output_path should be None if not outputting, or Some(output_path_argument)
pub(super) fn open_live_iso(
    input_path: &str,
    output_path: Option<Option<&String>>,
) -> Result<File> {
    // if output_path is Some(None), we're modifying in place, so we need to
    // open for writing
    OpenOptions::new()
        .read(true)
        .write(matches!(output_path, Some(None)))
        .open(&input_path)
        .with_context(|| format!("opening {}", &input_path))
}

pub(super) fn write_live_iso(
    iso: &IsoConfig,
    input: &mut File,
    output_path: Option<&String>,
) -> Result<()> {
    match output_path.map(|v| v.as_str()) {
        None => {
            // open_live_iso() opened input for writing
            iso.write(input)?;
        }
        Some("-") => {
            verify_stdout_not_tty()?;
            iso.stream(input, &mut io::stdout().lock())?;
        }
        Some(output_path) => {
            let output_dir = Path::new(output_path)
                .parent()
                .with_context(|| format!("no parent directory of {}", output_path))?;
            let mut output = tempfile::Builder::new()
                .prefix(".coreos-installer-temp-")
                .tempfile_in(output_dir)
                .context("creating temporary file")?;
            input.seek(SeekFrom::Start(0)).context("seeking input")?;
            copy(input, output.as_file_mut()).context("copying input to temporary file")?;
            iso.write(output.as_file_mut())?;
            output
                .persist_noclobber(&output_path)
                .map_err(|e| e.error)
                .with_context(|| format!("persisting output file to {}", output_path))?;
        }
    }
    Ok(())
}

/// If output_path is None, we write to stdout.  The caller is expected to
/// have called verify_stdout_not_tty() in this case.
pub(super) fn write_live_pxe(initrd: &Initrd, output_path: Option<&String>) -> Result<()> {
    let initrd = initrd.to_bytes()?;
    match output_path {
        Some(path) => write(path, &initrd).with_context(|| format!("writing {}", path)),
        None => {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            out.write_all(&initrd).context("writing output")?;
            out.flush().context("flushing output")
        }
    }
}

pub(super) fn copy_file_from_iso(
    iso: &mut IsoFs,
    file: &iso9660::File,
    output_path: &Path,
) -> Result<()> {
    let mut outf = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .with_context(|| format!("opening {}", output_path.display()))?;
    let mut bufw = BufWriter::with_capacity(BUFFER_SIZE, &mut outf);
    copy(&mut iso.read_file(file)?, &mut bufw)?;
    bufw.flush().context("flushing buffer")?;
    Ok(())
}

pub(super) fn verify_stdout_not_tty() -> Result<()> {
    if isatty(io::stdout().as_raw_fd()).context("checking if stdout is a TTY")? {
        bail!("Refusing to write binary data to terminal");
    }
    Ok(())
}

pub(super) fn filename(path: &str) -> Result<String> {
    Ok(Path::new(path)
        .file_name()
        .with_context(|| format!("missing filename in {}", path))?
        // path was originally a string
        .to_string_lossy()
        .into_owned())
}
