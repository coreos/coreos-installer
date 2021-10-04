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

use crate::cmdline::*;
use anyhow::{bail, Result};

use libcoreinst::blockdev::*;

pub fn verify_single_partition(config: &PartitionLabelConfig) -> Result<()> {
    // fail if we have more than 1 partition with boot label
    let devices = get_all_block_devices()?;
    let amount = count_partitions_with_label(&config.label, &devices.blockdevices);
    if amount != 1 {
        bail!(
            "System has {} partitions with '{}' label",
            amount,
            config.label
        );
    }
    Ok(())
}
