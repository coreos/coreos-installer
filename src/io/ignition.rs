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

use anyhow::{Context, Result};
use ignition_config::v3_3 as ign;

#[derive(Debug, Default)]
pub struct Ignition {
    config: ign::Config,
}

impl Ignition {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut json = serde_json::to_vec(&self.config).context("serializing Ignition config")?;
        json.push(b'\n');
        Ok(json)
    }
}
