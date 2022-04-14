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

//! Miscellaneous helper types.

use anyhow::{anyhow, Error, Result};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::default::Default;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::str::FromStr;

#[derive(Debug, PartialEq, Eq)]
pub enum PartitionFilter {
    Label(glob::Pattern),
    Index(Option<NonZeroU32>, Option<NonZeroU32>),
}

#[derive(Debug, DeserializeFromStr, SerializeDisplay, Clone, Copy, PartialEq, Eq)]
pub enum FetchRetries {
    Infinite,
    Finite(NonZeroU32),
    None,
}

impl FromStr for FetchRetries {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "infinite" => Ok(Self::Infinite),
            num => num
                .parse::<u32>()
                .map(|num| NonZeroU32::new(num).map(Self::Finite).unwrap_or(Self::None))
                .map_err(|e| anyhow!(e)),
        }
    }
}

impl fmt::Display for FetchRetries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "0"),
            Self::Finite(n) => write!(f, "{}", n),
            Self::Infinite => write!(f, "infinite"),
        }
    }
}

impl Default for FetchRetries {
    fn default() -> Self {
        Self::None
    }
}

/// A String wrapper that takes a parameterized type defining the default
/// value of the String.
#[derive(Debug, PartialEq, Eq)]
pub struct DefaultedString<S: DefaultString> {
    value: String,
    default: PhantomData<S>,
}

impl<S: DefaultString> DefaultedString<S> {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl<S: DefaultString> FromStr for DefaultedString<S> {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            value: s.to_string(),
            default: PhantomData,
        })
    }
}

impl<S: DefaultString> fmt::Display for DefaultedString<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl<S: DefaultString> Default for DefaultedString<S> {
    fn default() -> Self {
        Self {
            value: S::default(),
            default: PhantomData,
        }
    }
}

// SerializeDisplay derive apparently doesn't work with parameterized types
impl<S: DefaultString> Serialize for DefaultedString<S> {
    fn serialize<R>(&self, serializer: R) -> Result<R::Ok, R::Error>
    where
        R: serde::ser::Serializer,
    {
        serializer.serialize_str(&self.value)
    }
}

// DeserializeFromStr derive apparently doesn't work with parameterized types
impl<'de, S: DefaultString> Deserialize<'de> for DefaultedString<S> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        Ok(Self {
            value: String::deserialize(deserializer)?,
            default: PhantomData,
        })
    }
}

/// A default value for a DefaultedString.
pub trait DefaultString {
    fn default() -> String;
}

/// A default string of `uname -m`.
#[derive(Debug, PartialEq, Eq)]
pub struct Architecture {}
impl DefaultString for Architecture {
    fn default() -> String {
        nix::sys::utsname::uname().machine().to_string()
    }
}

/// The default path to NetworkManager connection files.
#[derive(Debug, PartialEq, Eq)]
pub struct NetworkDir {}
impl DefaultString for NetworkDir {
    fn default() -> String {
        "/etc/NetworkManager/system-connections/".into()
    }
}

pub(super) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}
