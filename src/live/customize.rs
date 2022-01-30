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

//! Infrastructure for high-level ISO/PXE customizations

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs::read;

use crate::cmdline::*;
use crate::io::*;
use crate::iso9660::{self, IsoFs};

use super::embed::{INITRD_IGNITION_PATH, INITRD_NETWORK_DIR};
use super::util::filename;

pub(super) const INITRD_FEATURES_PATH: &str = "etc/coreos/features.json";

const COREOS_ISO_FEATURES_PATH: &str = "COREOS/FEATURES.JSO";

/// CoreOS feature flags in /etc/coreos/features.json in the live initramfs
/// and /coreos/features.json in the live ISO.  Written by
/// cosa buildextend-live.
#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct OsFeatures {
    /// Installer reads config files from /etc/coreos/installer.d
    pub installer_config: bool,
    /// Live initrd reads NM keyfiles from /etc/coreos-firstboot-network
    pub live_initrd_network: bool,
}

impl OsFeatures {
    pub fn for_iso(iso: &mut IsoFs) -> Result<Self> {
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
pub(super) struct LiveInitrd {
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
    pub fn from_common(common: &CommonCustomizeConfig, features: OsFeatures) -> Result<Self> {
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

    pub fn dest_ignition(&mut self, path: &str) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        let (config, warnings) = ignition_config::Config::parse_slice(&data)
            .with_context(|| format!("parsing Ignition config {}", path))?;
        for warning in warnings {
            eprintln!("Warning parsing {}: {}", path, warning);
        }
        self.user_dest.push(config);
        Ok(())
    }

    pub fn dest_device(&mut self, device: &str) -> Result<()> {
        self.installer
            .get_or_insert_with(Default::default)
            .dest_device = Some(device.into());
        Ok(())
    }

    pub fn dest_karg_append(&mut self, arg: &str) {
        self.installer
            .get_or_insert_with(Default::default)
            .append_karg
            .push(arg.into());
    }

    pub fn dest_karg_delete(&mut self, arg: &str) {
        self.installer
            .get_or_insert_with(Default::default)
            .delete_karg
            .push(arg.into());
    }

    pub fn network_keyfile(&mut self, path: &str) -> Result<()> {
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

    pub fn ignition_ca(&mut self, path: &str) -> Result<()> {
        let data = read(path).with_context(|| format!("reading {}", path))?;
        self.live.add_ca(&data)?;
        self.dest_ca.push(data);
        Ok(())
    }

    pub fn pre_install(&mut self, path: &str) -> Result<()> {
        self.install_hook(
            path,
            "pre",
            "After=coreos-installer-pre.target\nBefore=coreos-installer.service",
            "coreos-installer.service",
        )
    }

    pub fn post_install(&mut self, path: &str) -> Result<()> {
        self.install_hook(
            path,
            "post",
            "After=coreos-installer.service\nBefore=coreos-installer.target",
            "coreos-installer.target",
        )
    }

    #[allow(clippy::format_in_format_args)]
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

    pub fn installer_config(&mut self, path: &str) -> Result<()> {
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

    pub fn live_config(&mut self, path: &str) -> Result<()> {
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

    pub fn into_initrd(mut self) -> Result<Initrd> {
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
