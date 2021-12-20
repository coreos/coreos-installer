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
use openat_ext::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{create_dir_all, read, write, File, OpenOptions};
use std::io::{self, copy, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::cmdline::*;
use crate::io::*;
use crate::iso9660::{self, IsoFs};
use crate::miniso;

mod embed;

use self::embed::*;

const INITRD_LIVE_STAMP_PATH: &str = "etc/coreos-live-initramfs";
const INITRD_FEATURES_PATH: &str = "etc/coreos/features.json";
const COREOS_ISO_FEATURES_PATH: &str = "COREOS/FEATURES.JSO";
const COREOS_ISO_PXEBOOT_DIR: &str = "IMAGES/PXEBOOT";
const COREOS_ISO_ROOTFS_IMG: &str = "IMAGES/PXEBOOT/ROOTFS.IMG";
const COREOS_ISO_MINISO_FILE: &str = "COREOS/MINISO.DAT";

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
        header: false,
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
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if config.header {
        out.write_all(&iso.initrd_header_json()?)
            .context("failed to write header")?;
    } else {
        if !iso.have_ignition() {
            bail!("No embedded Ignition config.");
        }
        out.write_all(
            iso.initrd()
                .get(INITRD_IGNITION_PATH)
                .context("couldn't find Ignition config in archive")?,
        )
        .context("writing output")?;
        out.flush().context("flushing output")?;
    }
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

    iso.set_kargs(&iso.kargs_default()?.to_string())?;

    write_live_iso(&iso, &mut iso_file, config.output.as_ref())
}

pub fn iso_kargs_show(config: IsoKargsShowConfig) -> Result<()> {
    let mut iso_file = open_live_iso(&config.input, None)?;
    let iso = IsoConfig::for_file(&mut iso_file)?;
    if config.header {
        io::stdout()
            .lock()
            .write_all(&iso.kargs_header_json()?)
            .context("failed to write header")?;
    } else {
        let kargs = if config.default {
            iso.kargs_default()?
        } else {
            iso.kargs()?
        };
        println!("{}", kargs);
    }
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

// output_path should be None if not outputting, or Some(output_path_argument)
fn open_live_iso(input_path: &str, output_path: Option<Option<&String>>) -> Result<File> {
    // if output_path is Some(None), we're modifying in place, so we need to
    // open for writing
    OpenOptions::new()
        .read(true)
        .write(matches!(output_path, Some(None)))
        .open(&input_path)
        .with_context(|| format!("opening {}", &input_path))
}

fn write_live_iso(iso: &IsoConfig, input: &mut File, output_path: Option<&String>) -> Result<()> {
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
            input
                .copy_to(output.as_file_mut())
                .context("copying input to temporary file")?;
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
fn write_live_pxe(initrd: &Initrd, output_path: Option<&String>) -> Result<()> {
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

/// CoreOS feature flags in /etc/coreos/features.json in the live initramfs
/// and /coreos/features.json in the live ISO.  Written by
/// cosa buildextend-live.
#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct OsFeatures {
    /// Installer reads config files from /etc/coreos/installer.d
    installer_config: bool,
    /// Live initrd reads NM keyfiles from /etc/coreos-firstboot-network
    live_initrd_network: bool,
}

impl OsFeatures {
    fn for_iso(iso: &mut IsoFs) -> Result<Self> {
        match iso.get_path(COREOS_ISO_FEATURES_PATH) {
            Ok(record) => serde_json::from_reader(
                iso.read_file(&record.try_into_file()?)
                    .context("reading OS features")?,
            )
            .context("parsing OS features"),
            Err(e) if e.is::<iso9660::NotFound>() => Ok(Self::default()),
            Err(e) => Err(e).context("looking up OS features"),
        }
    }
}

#[derive(Default)]
struct LiveInitrd {
    /// OS features
    features: OsFeatures,

    /// The initrd for the live system
    initrd: Initrd,
    /// The Ignition config for the live system
    live: Ignition,
    /// The Ignition config for the destination system
    dest: Option<Ignition>,
    /// User-supplied Ignition configs for the dest system, which might be
    /// merged into the dest config or might become the dest config
    user_dest: Vec<ignition_config::Config>,
    /// The coreos-installer config for our own parameters, excluding custom
    /// configs supplied by the user
    installer: Option<InstallConfig>,
    /// Have the installer copy network configs, if we are running it
    installer_copy_network: bool,
    /// Ignition CAs for the dest system, if it has an Ignition config
    dest_ca: Vec<Vec<u8>>,

    /// Prefix for installer config filenames
    installer_serial: u32,
}

impl LiveInitrd {
    fn from_common(common: &CommonCustomizeConfig, features: OsFeatures) -> Result<Self> {
        let mut conf = Self {
            features,
            ..Default::default()
        };

        for path in &common.dest_ignition {
            conf.dest_ignition(path)?;
        }
        if let Some(path) = &common.dest_device {
            conf.dest_device(path)?;
        }
        for arg in &common.dest_karg_append {
            conf.dest_karg_append(arg);
        }
        for arg in &common.dest_karg_delete {
            conf.dest_karg_delete(arg);
        }
        for path in &common.network_keyfile {
            conf.network_keyfile(path)?;
        }
        for path in &common.ignition_ca {
            conf.ignition_ca(path)?;
        }
        for path in &common.pre_install {
            conf.pre_install(path)?;
        }
        for path in &common.post_install {
            conf.post_install(path)?;
        }
        for path in &common.installer_config {
            conf.installer_config(path)?;
        }
        for path in &common.live_ignition {
            conf.live_config(path)?;
        }

        Ok(conf)
    }

    fn dest_ignition(&mut self, path: &str) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        let (config, warnings) = ignition_config::Config::parse_slice(&data)
            .with_context(|| format!("parsing Ignition config {}", path))?;
        for warning in warnings {
            eprintln!("Warning parsing {}: {}", path, warning);
        }
        self.user_dest.push(config);
        Ok(())
    }

    fn dest_device(&mut self, device: &str) -> Result<()> {
        self.installer
            .get_or_insert_with(Default::default)
            .dest_device = Some(device.into());
        Ok(())
    }

    fn dest_karg_append(&mut self, arg: &str) {
        self.installer
            .get_or_insert_with(Default::default)
            .append_karg
            .push(arg.into());
    }

    fn dest_karg_delete(&mut self, arg: &str) {
        self.installer
            .get_or_insert_with(Default::default)
            .delete_karg
            .push(arg.into());
    }

    fn network_keyfile(&mut self, path: &str) -> Result<()> {
        if !self.features.live_initrd_network {
            bail!("This OS image does not support customizing network settings.");
        }
        let data = read(path).with_context(|| format!("reading {}", path))?;
        let name = filename(path)?;
        let path = format!("{}/{}", INITRD_NETWORK_DIR, name);
        if self.initrd.get(&path).is_some() {
            bail!("config already specifies keyfile {}", name);
        }
        self.initrd.add(&path, data);
        self.installer_copy_network = true;
        Ok(())
    }

    fn ignition_ca(&mut self, path: &str) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        self.live.add_ca(&data)?;
        self.dest_ca.push(data);
        Ok(())
    }

    fn pre_install(&mut self, path: &str) -> Result<()> {
        self.install_hook(
            path,
            "pre",
            "After=coreos-installer-pre.target\nBefore=coreos-installer.service",
            "coreos-installer.service",
        )
    }

    fn post_install(&mut self, path: &str) -> Result<()> {
        self.install_hook(
            path,
            "post",
            "After=coreos-installer.service\nBefore=coreos-installer.target",
            "coreos-installer.target",
        )
    }

    fn install_hook(
        &mut self,
        path: &str,
        typ: &str,
        deps: &str,
        install_target: &str,
    ) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        let name = filename(path)?;
        self.live.add_file(
            format!("/usr/local/bin/{}-install-{}", typ, name),
            &data,
            0o700,
        )?;
        self.live.add_unit(
            format!("{}-install-{}.service", typ, name),
            format!(
                "# Generated by coreos-installer {{iso|pxe}} customize

[Unit]
Description={typ_title}-Install Script ({name})
Documentation=https://coreos.github.io/coreos-installer/customizing-install/
{deps}

[Service]
Type=oneshot
ExecStart=/usr/local/bin/{typ}-install-{name}
RemainAfterExit=true
StandardOutput=kmsg+console
StandardError=kmsg+console

[Install]
RequiredBy={install_target}",
                name = name,
                typ = typ,
                typ_title = format!("{}{}", typ[..1].to_uppercase(), &typ[1..]),
                deps = deps,
                install_target = install_target
            ),
            true,
        )
    }

    fn installer_config(&mut self, path: &str) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        // we don't validate but at least we parse
        serde_yaml::from_slice::<InstallConfig>(&data)
            .with_context(|| format!("parsing installer config {}", path))?;
        self.installer_config_bytes(&filename(path)?, &data)
    }

    fn installer_config_bytes(&mut self, filename: &str, data: &[u8]) -> Result<()> {
        if !self.features.installer_config {
            bail!("This OS image does not support customizing installer configuration.");
        }
        self.live.add_file(
            format!(
                "/etc/coreos/installer.d/{:04}-{}",
                self.installer_serial, filename
            ),
            data,
            0o600,
        )?;
        self.installer_serial += 1;
        Ok(())
    }

    fn live_config(&mut self, path: &str) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        // we don't validate but at least we parse
        let (config, warnings) = ignition_config::Config::parse_slice(&data)
            .with_context(|| format!("parsing Ignition config {}", path))?;
        for warning in warnings {
            eprintln!("Warning parsing {}: {}", path, warning);
        }
        self.live
            .merge_config(&config)
            .with_context(|| format!("merging Ignition config {}", path))
    }

    fn into_initrd(mut self) -> Result<Initrd> {
        if self.dest.is_some() || !self.user_dest.is_empty() {
            // Embed dest config in live and installer configs

            // We now know we'll have a dest config, so add CAs to it
            for ca in self.dest_ca.drain(..) {
                self.dest.get_or_insert_with(Default::default).add_ca(&ca)?;
            }

            let data = if self.dest.is_none() && self.user_dest.len() == 1 {
                // Special case: the user supplied exactly one dest config
                // and we didn't add any dest config directives of our own.
                // Avoid another level of wrapping by embedding the user's
                // dest config directly.
                let mut buf = serde_json::to_vec(&self.user_dest.pop().unwrap())
                    .context("serializing dest Ignition config")?;
                buf.push(b'\n');
                buf
            } else {
                let dest = self.dest.get_or_insert_with(Default::default);
                for user_dest in self.user_dest.drain(..) {
                    dest.merge_config(&user_dest)?;
                }
                dest.to_bytes()?
            };
            let conf = self.installer.get_or_insert_with(Default::default);
            assert!(conf.ignition_file.is_none());
            let dest_path = "/etc/coreos/dest.ign";
            self.live.add_file(dest_path.into(), &data, 0o600)?;
            conf.ignition_file = Some(dest_path.into());
        }

        if self.installer_serial > 0 || self.installer.is_some() {
            // The installer will run; apply deferred settings
            if let Some(device) = self.installer.as_ref().and_then(|c| c.dest_device.as_ref()) {
                eprintln!(
                    "Boot media will automatically install to {} without confirmation.",
                    device
                );
            } else {
                eprintln!("Boot media will automatically run installer.");
            }
            if self.installer_copy_network {
                self.installer
                    .get_or_insert_with(Default::default)
                    .copy_network = true;
            }
        }

        if let Some(conf) = self.installer.take() {
            // Embed installer config in live config
            self.installer_config_bytes(
                "customize.yaml",
                &serde_yaml::to_vec(&conf).context("serializing installer config")?,
            )?;
        }

        // Embed live config in initrd
        self.initrd.add(INITRD_IGNITION_PATH, self.live.to_bytes()?);
        Ok(self.initrd)
    }
}

#[derive(Serialize)]
struct IsoInspectOutput {
    header: IsoFs,
    records: Vec<String>,
}

pub fn iso_inspect(config: IsoInspectConfig) -> Result<()> {
    let mut iso = IsoFs::from_file(open_live_iso(&config.input, None)?)?;
    let records = iso
        .walk()?
        .map(|r| r.map(|(path, _)| path))
        .collect::<Result<Vec<String>>>()
        .context("while walking ISO filesystem")?;
    let inspect_out = IsoInspectOutput {
        header: iso,
        records,
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, &inspect_out)
        .context("failed to serialize ISO metadata")?;
    out.write_all(b"\n").context("failed to write newline")?;
    Ok(())
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

fn copy_file_from_iso(iso: &mut IsoFs, file: &iso9660::File, output_path: &Path) -> Result<()> {
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

pub fn iso_pack_minimal_iso(config: IsoExtractPackMinimalIsoConfig) -> Result<()> {
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

fn verify_stdout_not_tty() -> Result<()> {
    if isatty(io::stdout().as_raw_fd()).context("checking if stdout is a TTY")? {
        bail!("Refusing to write binary data to terminal");
    }
    Ok(())
}

fn filename(path: &str) -> Result<String> {
    Ok(Path::new(path)
        .file_name()
        .with_context(|| format!("missing filename in {}", path))?
        // path was originally a string
        .to_string_lossy()
        .into_owned())
}
