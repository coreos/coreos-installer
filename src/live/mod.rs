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
use lazy_static::lazy_static;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{create_dir_all, read, File, OpenOptions};
use std::io::{self, copy, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use crate::cmdline::*;
use crate::io::*;
use crate::iso9660::{self, IsoFs};
use crate::miniso;
use crate::util::set_die_on_sigpipe;

mod customize;
mod embed;
mod util;

use self::customize::*;
use self::embed::*;
use self::util::*;

const INITRD_LIVE_STAMP_PATH: &str = "etc/coreos-live-initramfs";
const COREOS_ISO_PXEBOOT_DIR: &str = "IMAGES/PXEBOOT";
const COREOS_ISO_ROOTFS_IMG: &str = "IMAGES/PXEBOOT/ROOTFS.IMG";
const COREOS_ISO_MINISO_FILE: &str = "COREOS/MINISO.DAT";

lazy_static! {
    static ref ALL_GLOB: GlobMatcher = GlobMatcher::new(&["*"]).unwrap();
}

pub fn iso_embed(config: IsoEmbedConfig) -> Result<()> {
    eprintln!("`iso embed` is deprecated; use `iso ignition embed`.  Continuing.");
    iso_ignition_embed(IsoIgnitionEmbedConfig {
        force: config.force,
        ignition_file: config.config,
        output: config.output,
        input: config.input,
    })
}

pub fn iso_show(config: IsoShowConfig) -> Result<()> {
    eprintln!("`iso show` is deprecated; use `iso ignition show`.  Continuing.");
    iso_ignition_show(IsoIgnitionShowConfig {
        input: config.input,
    })
}

pub fn iso_remove(config: IsoRemoveConfig) -> Result<()> {
    eprintln!("`iso remove` is deprecated; use `iso ignition remove`.  Continuing.");
    iso_ignition_remove(IsoIgnitionRemoveConfig {
        output: config.output,
        input: config.input,
    })
}

pub fn iso_ignition_embed(config: IsoIgnitionEmbedConfig) -> Result<()> {
    let ignition = match &config.ignition_file {
        Some(ignition_path) => {
            read(ignition_path).with_context(|| format!("reading {}", ignition_path))?
        }
        None => {
            let mut data = Vec::new();
            io::stdin()
                .lock()
                .read_to_end(&mut data)
                .context("reading stdin")?;
            data
        }
    };

    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    if !config.force && iso.have_ignition() {
        bail!("This ISO image already has an embedded Ignition config; use -f to force.");
    }

    iso.initrd_mut().add(INITRD_IGNITION_PATH, ignition);

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_ignition_show(config: IsoIgnitionShowConfig) -> Result<()> {
    set_die_on_sigpipe()?;
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    if !iso.have_ignition() {
        bail!("No embedded Ignition config.");
    }
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(
        iso.initrd()
            .get(INITRD_IGNITION_PATH)
            .context("couldn't find Ignition config in archive")?,
    )
    .context("writing output")?;
    out.flush().context("flushing output")?;
    Ok(())
}

pub fn iso_ignition_remove(config: IsoIgnitionRemoveConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    iso.initrd_mut().remove(INITRD_IGNITION_PATH);

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_network_embed(config: IsoNetworkEmbedConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso_fs = IsoFs::from_file(iso_file.try_clone().context("cloning file")?)
        .context("parsing ISO9660 image")?;
    let mut iso = IsoConfig::for_iso(&mut iso_fs)?;

    if !OsFeatures::for_iso(&mut iso_fs)?.live_initrd_network {
        bail!("This OS image does not support customizing network settings.");
    }
    if !config.force && iso.have_network() {
        bail!("This ISO image already has embedded network settings; use -f to force.");
    }

    iso.remove_network();
    initrd_network_embed(iso.initrd_mut(), &config.keyfile)?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_network_extract(config: IsoNetworkExtractConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    initrd_network_extract(iso.initrd(), config.directory.as_ref())
}

pub fn iso_network_remove(config: IsoNetworkRemoveConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    iso.remove_network();

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn pxe_ignition_wrap(config: PxeIgnitionWrapConfig) -> Result<()> {
    if config.output.is_none() {
        verify_stdout_not_tty()?;
    }

    let ignition = match &config.ignition_file {
        Some(ignition_path) => {
            read(ignition_path).with_context(|| format!("reading {}", ignition_path))?
        }
        None => {
            let mut data = Vec::new();
            io::stdin()
                .lock()
                .read_to_end(&mut data)
                .context("reading stdin")?;
            data
        }
    };

    let mut initrd = Initrd::default();
    initrd.add(INITRD_IGNITION_PATH, ignition);

    write_live_pxe(&initrd, config.output.as_ref())
}

pub fn pxe_ignition_unwrap(config: PxeIgnitionUnwrapConfig) -> Result<()> {
    set_die_on_sigpipe()?;
    let stdin = io::stdin();
    let mut f: Box<dyn Read> = if let Some(path) = &config.input {
        Box::new(
            OpenOptions::new()
                .read(true)
                .open(path)
                .with_context(|| format!("opening {}", path))?,
        )
    } else {
        Box::new(stdin.lock())
    };
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(
        Initrd::from_reader_filtered(&mut f, &INITRD_IGNITION_GLOB)?
            .get(INITRD_IGNITION_PATH)
            .context("couldn't find Ignition config in archive")?,
    )
    .context("writing output")?;
    out.flush().context("flushing output")?;
    Ok(())
}

pub fn pxe_network_wrap(config: PxeNetworkWrapConfig) -> Result<()> {
    if config.output.is_none() {
        verify_stdout_not_tty()?;
    }

    let mut initrd = Initrd::default();
    initrd_network_embed(&mut initrd, &config.keyfile)?;

    write_live_pxe(&initrd, config.output.as_ref())
}

fn initrd_network_embed(initrd: &mut Initrd, keyfiles: &[String]) -> Result<()> {
    for path in keyfiles {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        let name = filename(path)?;
        let path = format!("{}/{}", INITRD_NETWORK_DIR, name);
        if initrd.get(&path).is_some() {
            bail!("multiple input files named '{}'", name);
        }
        initrd.add(&path, data);
    }
    Ok(())
}

pub fn pxe_network_unwrap(config: PxeNetworkUnwrapConfig) -> Result<()> {
    let stdin = io::stdin();
    let f: Box<dyn Read> = if let Some(path) = &config.input {
        Box::new(
            OpenOptions::new()
                .read(true)
                .open(path)
                .with_context(|| format!("opening {}", path))?,
        )
    } else {
        Box::new(stdin.lock())
    };
    initrd_network_extract(
        &Initrd::from_reader_filtered(f, &INITRD_NETWORK_GLOB)?,
        config.directory.as_ref(),
    )
}

fn initrd_network_extract(initrd: &Initrd, directory: Option<&String>) -> Result<()> {
    let files = initrd.find(&INITRD_NETWORK_GLOB);
    if files.is_empty() {
        bail!("No embedded network settings.");
    }
    if let Some(dir) = directory {
        create_dir_all(&dir)?;
        for (path, contents) in files {
            let path = Path::new(dir).join(filename(path)?);
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .with_context(|| format!("opening {}", path.display()))?
                .write_all(contents)
                .with_context(|| format!("writing {}", path.display()))?;
            println!("{}", path.display());
        }
    } else {
        set_die_on_sigpipe()?;
        for (i, (path, contents)) in files.iter().enumerate() {
            if i > 0 {
                println!();
            }
            println!("########## {} ##########", filename(path)?);
            io::stdout()
                .lock()
                .write_all(contents)
                .context("writing network settings to stdout")?;
        }
    }
    Ok(())
}

pub fn iso_kargs_modify(config: IsoKargsModifyConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    let kargs = KargsEditor::new()
        .append(&config.append)
        .replace(&config.replace)
        .delete(&config.delete)
        .apply_to(iso.kargs()?)?;
    iso.set_kargs(&kargs)?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_reset(config: IsoKargsResetConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    #[allow(clippy::unnecessary_to_owned)]
    iso.set_kargs(&iso.kargs_default()?.to_string())?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_show(config: IsoKargsShowConfig) -> Result<()> {
    set_die_on_sigpipe()?;
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    let kargs = if config.default {
        iso.kargs_default()?
    } else {
        iso.kargs()?
    };
    println!("{}", kargs);
    Ok(())
}

pub fn iso_customize(config: IsoCustomizeConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso_fs = IsoFs::from_file(iso_file.try_clone().context("cloning file")?)
        .context("parsing ISO9660 image")?;
    let mut iso = IsoConfig::for_iso(&mut iso_fs)?;

    if !config.force
        && (iso.have_ignition()
            || iso.have_network()
            || (iso.kargs_supported() && iso.kargs()? != iso.kargs_default()?))
    {
        bail!("This ISO image is already customized; use -f to force.");
    }

    let live = LiveInitrd::from_common(&config.common, OsFeatures::for_iso(&mut iso_fs)?)?;
    *iso.initrd_mut() = live.into_initrd()?;

    if [
        &config.live_karg_append,
        &config.live_karg_replace,
        &config.live_karg_delete,
    ]
    .iter()
    .any(|v| !v.is_empty())
    {
        if !iso.kargs_supported() {
            bail!("This OS image does not support customizing live kernel arguments.");
        }
        let kargs = KargsEditor::new()
            .append(&config.live_karg_append)
            .replace(&config.live_karg_replace)
            .delete(&config.live_karg_delete)
            .apply_to(iso.kargs_default()?)?;
        iso.set_kargs(&kargs)?;
    }

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_reset(config: IsoResetConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, Some(config.output.as_ref()))?;
    let mut iso = IsoConfig::for_file(&mut iso_file)?;

    *iso.initrd_mut() = Initrd::default();
    if iso.kargs_supported() {
        #[allow(clippy::unnecessary_to_owned)]
        iso.set_kargs(&iso.kargs_default()?.to_string())?;
    };

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn pxe_customize(config: PxeCustomizeConfig) -> Result<()> {
    // open input and set up output
    let mut input = BufReader::with_capacity(
        BUFFER_SIZE,
        OpenOptions::new()
            .read(true)
            .open(&config.input)
            .with_context(|| format!("opening {}", &config.input))?,
    );
    let mut tempfile = match &*config.output {
        "-" => {
            verify_stdout_not_tty()?;
            None
        }
        path => {
            let dir = Path::new(path)
                .parent()
                .with_context(|| format!("no parent directory of {}", path))?;
            let tempfile = tempfile::Builder::new()
                .prefix(".coreos-installer-temp-")
                .tempfile_in(dir)
                .context("creating temporary file")?;
            Some(tempfile)
        }
    };

    // copy and check base initrd
    let filter = GlobMatcher::new(&[
        INITRD_LIVE_STAMP_PATH,
        INITRD_FEATURES_PATH,
        INITRD_IGNITION_PATH,
        &format!("{}/*", INITRD_NETWORK_DIR),
    ])
    .unwrap();
    let base_initrd = match &*config.output {
        "-" => {
            Initrd::from_reader_filtered(TeeReader::new(&mut input, io::stdout().lock()), &filter)
                .context("reading/copying input initrd")?
        }
        _ => Initrd::from_reader_filtered(
            TeeReader::new(&mut input, tempfile.as_mut().unwrap()),
            &filter,
        )
        .context("reading/copying input initrd")?,
    };
    if base_initrd.get(INITRD_LIVE_STAMP_PATH).is_none() {
        bail!("not a CoreOS live initramfs image");
    }
    if base_initrd.get(INITRD_IGNITION_PATH).is_some()
        || !base_initrd.find(&INITRD_NETWORK_GLOB).is_empty()
    {
        bail!("input is already customized");
    }
    let features = match base_initrd.get(INITRD_FEATURES_PATH) {
        Some(json) => serde_json::from_slice::<OsFeatures>(json).context("parsing OS features")?,
        None => OsFeatures::default(),
    };

    let live = LiveInitrd::from_common(&config.common, features)?;
    let initrd = live.into_initrd()?;

    // append customizations to output
    let do_write = |writer: &mut dyn Write| -> Result<()> {
        let mut buf = BufWriter::with_capacity(BUFFER_SIZE, writer);
        buf.write_all(&initrd.to_bytes()?)
            .context("writing initrd")?;
        buf.flush().context("flushing initrd")
    };
    match &*config.output {
        "-" => do_write(&mut io::stdout().lock()),
        path => {
            let mut tempfile = tempfile.unwrap();
            do_write(tempfile.as_file_mut())?;
            tempfile
                .persist_noclobber(&path)
                .map_err(|e| e.error)
                .with_context(|| format!("persisting output file to {}", path))?;
            Ok(())
        }
    }
}

#[derive(Serialize)]
struct DevShowIsoOutput {
    header: IsoFs,
    records: Vec<String>,
}

pub fn dev_show_iso(config: DevShowIsoConfig) -> Result<()> {
    set_die_on_sigpipe()?;
    let mut iso_file = open_live_iso(&config.input, None)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if config.ignition || config.kargs {
        let iso = IsoConfig::for_file(&mut iso_file)?;
        let data = if config.ignition {
            iso.initrd_header_json()?
        } else {
            iso.kargs_header_json()?
        };
        out.write_all(&data).context("failed to write header")?;
    } else {
        let mut iso = IsoFs::from_file(iso_file)?;
        let records = iso
            .walk()?
            .map(|r| r.map(|(path, _)| path))
            .collect::<Result<Vec<String>>>()
            .context("while walking ISO filesystem")?;
        let info = DevShowIsoOutput {
            header: iso,
            records,
        };

        serde_json::to_writer_pretty(&mut out, &info)
            .context("failed to serialize ISO metadata")?;
        out.write_all(b"\n").context("failed to write newline")?;
    }
    Ok(())
}

pub fn dev_show_initrd(config: DevShowInitrdConfig) -> Result<()> {
    set_die_on_sigpipe()?;
    let initrd = read_initrd(&config.input, &config.filter)?;
    for path in initrd.find(&ALL_GLOB).keys() {
        println!("{}", path);
    }
    Ok(())
}

pub fn dev_extract_initrd(config: DevExtractInitrdConfig) -> Result<()> {
    let initrd = read_initrd(&config.input, &config.filter)?;
    let base_path = Path::new(&config.directory);
    for (path, contents) in initrd.find(&ALL_GLOB) {
        if Path::new(path)
            .components()
            .any(|c| matches!(c, Component::RootDir | Component::ParentDir))
        {
            bail!("path {} contains path traversal", path);
        }
        let out_path = base_path.join(path);
        if config.verbose {
            println!("{}", out_path.display());
        }
        let out_parent = out_path
            .parent()
            .with_context(|| format!("finding parent of {}", out_path.display()))?;
        create_dir_all(out_parent).with_context(|| format!("creating {}", out_parent.display()))?;
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&out_path)
            .with_context(|| format!("opening {}", out_path.display()))?
            .write_all(contents)
            .with_context(|| format!("writing {}", out_path.display()))?;
    }
    Ok(())
}

fn read_initrd(path: &str, filter: &[String]) -> Result<Initrd> {
    let filter = if filter.is_empty() {
        vec!["*"]
    } else {
        filter.iter().map(String::as_str).collect()
    };
    let filter = GlobMatcher::new(&filter).context("parsing glob patterns")?;
    match path {
        "-" => Initrd::from_reader_filtered(io::stdin().lock(), &filter),
        path => Initrd::from_reader_filtered(
            OpenOptions::new()
                .read(true)
                .open(path)
                .with_context(|| format!("opening {}", path))?,
            &filter,
        ),
    }
    .context("decoding initrd")
}

pub fn iso_extract_pxe(config: IsoExtractPxeConfig) -> Result<()> {
    let mut iso = IsoFs::from_file(open_live_iso(&config.input, None)?)?;
    let pxeboot = iso
        .get_path(COREOS_ISO_PXEBOOT_DIR)
        .context("Unrecognized CoreOS ISO image.")?
        .try_into_dir()?;
    create_dir_all(&config.output_dir)?;

    let base = {
        // this can't be None since we successfully opened the live ISO at the location
        let mut s = Path::new(&config.input).file_stem().unwrap().to_os_string();
        s.push("-");
        s
    };

    for record in iso.list_dir(&pxeboot)? {
        match record? {
            iso9660::DirectoryRecord::Directory(_) => continue,
            iso9660::DirectoryRecord::File(file) => {
                let filename = {
                    let mut s = base.clone();
                    s.push(file.name.to_lowercase());
                    s
                };
                let path = Path::new(&config.output_dir).join(&filename);
                println!("{}", path.display());
                copy_file_from_iso(&mut iso, &file, &path)?;
            }
        }
    }
    Ok(())
}

pub fn iso_extract_minimal_iso(config: IsoExtractMinimalIsoConfig) -> Result<()> {
    // Note we don't support overwriting the input ISO. Unlike other commands, this operation is
    // non-reversible, so let's make it harder for users to shoot themselves in the foot.
    let mut full_iso = IsoFs::from_file(open_live_iso(&config.input, None)?)?;

    // For now, we require the full ISO to be completely vanilla. Otherwise, the hashes won't
    // match.
    let iso = IsoConfig::for_iso(&mut full_iso)?;
    if !iso.initrd().is_empty() || iso.kargs()? != iso.kargs_default()? {
        bail!("Cannot operate on ISO with embedded customizations.\nReset it with `coreos-installer iso reset` and try again.");
    }

    // do this early so we exit immediately if stdout is a TTY
    let output_dir: PathBuf = if &config.output == "-" {
        verify_stdout_not_tty()?;
        std::env::temp_dir()
    } else {
        Path::new(&config.output)
            .parent()
            .with_context(|| format!("no parent directory of {}", &config.output))?
            .into()
    };

    if let Some(path) = &config.output_rootfs {
        let rootfs = full_iso
            .get_path(COREOS_ISO_ROOTFS_IMG)
            .with_context(|| format!("looking up '{}'", COREOS_ISO_ROOTFS_IMG))?
            .try_into_file()?;
        copy_file_from_iso(&mut full_iso, &rootfs, Path::new(path))?;
    }

    let miniso_data_file = match full_iso.get_path(COREOS_ISO_MINISO_FILE) {
        Ok(record) => record.try_into_file()?,
        Err(e) if e.is::<iso9660::NotFound>() => {
            bail!("This ISO image does not support extracting a minimal ISO.")
        }
        Err(e) => {
            return Err(e).with_context(|| format!("looking up '{}'", COREOS_ISO_MINISO_FILE))
        }
    };

    let data = {
        let mut f = full_iso.read_file(&miniso_data_file)?;
        miniso::Data::deserialize(&mut f).context("reading miniso data file")?
    };
    let mut outf = tempfile::Builder::new()
        .prefix(".coreos-installer-temp-")
        .tempfile_in(&output_dir)
        .context("creating temporary file")?;
    data.unxzpack(full_iso.as_file()?, &mut outf)
        .context("unpacking miniso")?;
    outf.seek(SeekFrom::Start(0))
        .context("seeking back to start of miniso tempfile")?;

    modify_miniso_kargs(outf.as_file_mut(), config.rootfs_url.as_ref())
        .context("modifying miniso kernel args")?;

    if &config.output == "-" {
        copy(&mut outf, &mut io::stdout().lock()).context("writing output")?;
    } else {
        outf.persist_noclobber(&config.output)
            .map_err(|e| e.error)?;
    }

    Ok(())
}

pub fn pack_minimal_iso(config: PackMinimalIsoConfig) -> Result<()> {
    let mut full_iso = IsoFs::from_file(open_live_iso(&config.full, Some(None))?)?;
    let mut minimal_iso = IsoFs::from_file(open_live_iso(&config.minimal, None)?)?;

    let full_files = collect_iso_files(&mut full_iso)
        .with_context(|| format!("collecting files from {}", &config.full))?;
    let minimal_files = collect_iso_files(&mut minimal_iso)
        .with_context(|| format!("collecting files from {}", &config.minimal))?;
    if full_files.is_empty() {
        bail!("No files found in {}", &config.full);
    } else if minimal_files.is_empty() {
        bail!("No files found in {}", &config.minimal);
    }

    eprintln!("Packing minimal ISO");
    let (data, matches, skipped, written, written_compressed) =
        miniso::Data::xzpack(minimal_iso.as_file()?, &full_files, &minimal_files)
            .context("packing miniso")?;
    eprintln!("Matched {} files of {}", matches, minimal_files.len());

    eprintln!("Total bytes skipped: {}", skipped);
    eprintln!("Total bytes written: {}", written);
    eprintln!("Total bytes written (compressed): {}", written_compressed);

    eprintln!("Verifying that packed image matches digest");
    data.unxzpack(full_iso.as_file()?, std::io::sink())
        .context("unpacking miniso for verification")?;

    let miniso_entry = full_iso
        .get_path(COREOS_ISO_MINISO_FILE)
        .with_context(|| format!("looking up '{}'", COREOS_ISO_MINISO_FILE))?
        .try_into_file()?;
    let mut w = full_iso.overwrite_file(&miniso_entry)?;
    data.serialize(&mut w).context("writing miniso data file")?;
    w.flush().context("flushing full ISO")?;

    if config.consume {
        std::fs::remove_file(&config.minimal)
            .with_context(|| format!("consuming {}", &config.minimal))?;
    }

    eprintln!("Packing successful!");
    Ok(())
}

fn collect_iso_files(iso: &mut IsoFs) -> Result<HashMap<String, iso9660::File>> {
    iso.walk()?
        .filter_map(|r| match r {
            Err(e) => Some(Err(e)),
            Ok((s, iso9660::DirectoryRecord::File(f))) => Some(Ok((s, f))),
            Ok(_) => None,
        })
        .collect::<Result<HashMap<String, iso9660::File>>>()
        .context("while walking ISO filesystem")
}

fn modify_miniso_kargs(f: &mut File, rootfs_url: Option<&String>) -> Result<()> {
    let mut iso = IsoFs::from_file(f.try_clone().context("cloning a file")?)?;
    let mut cfg = IsoConfig::for_file(f)?;

    let kargs = cfg.kargs()?;

    // same disclaimer as `modify_kargs()` here re. whitespace/quoting
    let liveiso_karg = kargs
        .split_ascii_whitespace()
        .find(|&karg| karg.starts_with("coreos.liveiso="))
        .context("minimal ISO does not have coreos.liveiso= karg")?
        .to_string();

    let new_default_kargs = KargsEditor::new().delete(&[liveiso_karg]).apply_to(kargs)?;
    cfg.set_kargs(&new_default_kargs)?;

    if let Some(url) = rootfs_url {
        if url.split_ascii_whitespace().count() > 1 {
            bail!("forbidden whitespace found in '{}'", url);
        }
        let final_kargs = KargsEditor::new()
            .append(&[format!("coreos.live.rootfs_url={}", url)])
            .apply_to(&new_default_kargs)?;

        cfg.set_kargs(&final_kargs)?;
    }

    // update kargs
    write_live_iso(&cfg, f, None)?;

    // also modify the default kargs because we don't want `coreos-installer iso kargs reset` to
    // re-add `coreos.liveiso`
    set_default_kargs(&mut iso, new_default_kargs)
}
