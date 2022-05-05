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

use clap::ArgEnum;

pub mod dasd;
pub mod zipl;

pub use dasd::{dasd_try_get_sector_size, image_copy_s390x, prepare_dasd};
pub use zipl::chreipl;
pub use zipl::zipl;
mod eckd;
mod fba;

#[derive(ArgEnum, Clone, Debug)]
pub enum ZiplSecexMode {
    Auto,
    Enforce,
    Disable,
}
