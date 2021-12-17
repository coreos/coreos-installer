// Copyright 2021 Red Hat
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
use flate2::read::GzEncoder;
use flate2::Compression;
use ignition_config as ign_multi;
use ignition_config::v3_3 as ign;
use std::io::Read;

#[derive(Debug, Default)]
pub struct Ignition {
    config: ign::Config,
}

impl Ignition {
    pub fn merge_config(&mut self, config: &ign_multi::Config) -> Result<()> {
        let buf = serde_json::to_vec(config).context("serializing child Ignition config")?;
        self.config
            .ignition
            .config
            .get_or_insert_with(Default::default)
            .merge
            .get_or_insert_with(Default::default)
            .push(make_resource(&buf)?);
        Ok(())
    }

    pub fn add_file(&mut self, path: String, data: &[u8], mode: i64) -> Result<()> {
        // Perform the same alias check that Ignition config validation does.
        // This doesn't catch aliases known only at runtime, such as
        // /usr/local and /var/usrlocal.
        if self.have_path(&path) {
            bail!("config already specifies path {}", path);
        }
        self.config
            .storage
            .get_or_insert_with(Default::default)
            .files
            .get_or_insert_with(Default::default)
            .push(ign::File {
                contents: Some(make_resource(data)?),
                mode: Some(mode),
                ..ign::File::new(path)
            });
        Ok(())
    }

    pub fn add_unit(&mut self, name: String, contents: String, enabled: bool) -> Result<()> {
        let units = self
            .config
            .systemd
            .get_or_insert_with(Default::default)
            .units
            .get_or_insert_with(Default::default);
        if units.iter().any(|u| u.name == name) {
            bail!("config already specifies unit {}", name);
        }
        units.push(ign::Unit {
            contents: Some(contents),
            enabled: Some(enabled),
            ..ign::Unit::new(name)
        });
        Ok(())
    }

    pub fn add_ca(&mut self, data: &[u8]) -> Result<()> {
        self.config
            .ignition
            .security
            .get_or_insert_with(Default::default)
            .tls
            .get_or_insert_with(Default::default)
            .certificate_authorities
            .get_or_insert_with(Default::default)
            .push(make_resource(data)?);
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut json = serde_json::to_vec(&self.config).context("serializing Ignition config")?;
        json.push(b'\n');
        Ok(json)
    }

    fn have_path(&self, path: &str) -> bool {
        let storage = self.config.storage.clone().unwrap_or_default();
        storage
            .files
            .unwrap_or_default()
            .iter()
            .map(|f| &f.path)
            .chain(
                storage
                    .directories
                    .unwrap_or_default()
                    .iter()
                    .map(|d| &d.path),
            )
            .chain(storage.links.unwrap_or_default().iter().map(|l| &l.path))
            .any(|p| p == path)
    }
}

fn make_resource(data: &[u8]) -> Result<ign::Resource> {
    let mut compressed = Vec::new();
    GzEncoder::new(data, Compression::best()).read_to_end(&mut compressed)?;
    Ok(ign::Resource {
        source: Some(format!("data:;base64,{}", base64::encode(&compressed))),
        compression: Some("gzip".into()),
        ..Default::default()
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn duplicate_path() {
        let mut ignition = Ignition::default();
        ignition.add_file("/a/b".into(), &[], 0o755).unwrap();
        ignition.add_file("/a/b".into(), &[], 0o755).unwrap_err();
    }
}
