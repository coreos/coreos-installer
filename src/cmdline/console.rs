// Copyright 2022 Red Hat, Inc.
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

//! Helper types for console argument.

use anyhow::{bail, Context, Error, Result};
use lazy_static::lazy_static;
use regex::Regex;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::fmt;
use std::str::FromStr;

const KARG_PREFIX: &str = "console=";

#[derive(Clone, Debug, DeserializeFromStr, SerializeDisplay, PartialEq, Eq)]
pub enum Console {
    Graphical(GraphicalConsole),
    Serial(SerialConsole),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphicalConsole {
    device: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialConsole {
    prefix: String,
    port: u8,
    speed: u32,
    data_bits: u8,
    parity: Parity,
    // Linux console doesn't support stop bits
    // GRUB doesn't support RTS/CTS flow control
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Parity {
    None,
    Odd,
    Even,
}

impl Parity {
    fn for_grub(&self) -> &'static str {
        match self {
            Self::None => "no",
            Self::Odd => "odd",
            Self::Even => "even",
        }
    }

    fn for_karg(&self) -> &'static str {
        match self {
            Self::None => "n",
            Self::Odd => "o",
            Self::Even => "e",
        }
    }
}

impl Console {
    pub fn grub_terminal(&self) -> &'static str {
        match self {
            Self::Graphical(_) => "console",
            Self::Serial(_) => "serial",
        }
    }

    pub fn grub_command(&self) -> Option<String> {
        match self {
            Self::Graphical(_) => None,
            Self::Serial(c) => Some(format!(
                "serial --unit={} --speed={} --word={} --parity={}",
                c.port,
                c.speed,
                c.data_bits,
                c.parity.for_grub()
            )),
        }
    }

    pub fn karg(&self) -> String {
        format!("{KARG_PREFIX}{}", self)
    }
}

impl FromStr for Console {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // help the user with possible misunderstandings
        for prefix in [KARG_PREFIX, "/dev/"] {
            if s.starts_with(prefix) {
                bail!(r#"spec should not start with "{prefix}""#);
            }
        }

        // first, parse serial console parameters
        lazy_static! {
            static ref SERIAL_REGEX: Regex = Regex::new("^(?P<prefix>ttyS|ttyAMA)(?P<port>[0-9]+)(?:,(?P<speed>[0-9]+)(?:(?P<parity>n|o|e)(?P<data_bits>[5-8])?)?)?$").expect("compiling console regex");
        }
        if let Some(c) = SERIAL_REGEX.captures(s) {
            return Ok(Console::Serial(SerialConsole {
                prefix: c
                    .name("prefix")
                    .expect("prefix is mandatory")
                    .as_str()
                    .to_string(),
                port: c
                    .name("port")
                    .expect("port is mandatory")
                    .as_str()
                    .parse()
                    .context("couldn't parse port")?,
                speed: c
                    .name("speed")
                    .map(|v| v.as_str().parse().context("couldn't parse speed"))
                    .unwrap_or(Ok(9600))?,
                data_bits: c
                    .name("data_bits")
                    .map(|v| v.as_str().parse().expect("unexpected data bits"))
                    .unwrap_or(8),
                parity: match c.name("parity").map(|v| v.as_str()) {
                    // default
                    None => Parity::None,
                    Some("n") => Parity::None,
                    Some("e") => Parity::Even,
                    Some("o") => Parity::Odd,
                    _ => unreachable!(),
                },
            }));
        }

        // then try hardcoded strings for graphical consoles
        match s {
            "tty0" | "hvc0" | "ttysclp0" => Ok(Console::Graphical(GraphicalConsole {
                device: s.to_string(),
            })),
            _ => bail!("invalid or unsupported console argument"),
        }
    }
}

impl fmt::Display for Console {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graphical(c) => write!(f, "{}", c.device),
            Self::Serial(c) => write!(
                f,
                "{}{},{}{}{}",
                c.prefix,
                c.port,
                c.speed,
                c.parity.for_karg(),
                c.data_bits
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_console_args() {
        let cases = vec![
            ("tty0", "console=tty0", "console", None),
            ("hvc0", "console=hvc0", "console", None),
            ("ttysclp0", "console=ttysclp0", "console", None),
            (
                "ttyS1",
                "console=ttyS1,9600n8",
                "serial",
                Some("serial --unit=1 --speed=9600 --word=8 --parity=no"),
            ),
            (
                "ttyAMA1",
                "console=ttyAMA1,9600n8",
                "serial",
                Some("serial --unit=1 --speed=9600 --word=8 --parity=no"),
            ),
            (
                "ttyS1,1234567e5",
                "console=ttyS1,1234567e5",
                "serial",
                Some("serial --unit=1 --speed=1234567 --word=5 --parity=even"),
            ),
            (
                "ttyS2,5o",
                "console=ttyS2,5o8",
                "serial",
                Some("serial --unit=2 --speed=5 --word=8 --parity=odd"),
            ),
            (
                "ttyS3,17",
                "console=ttyS3,17n8",
                "serial",
                Some("serial --unit=3 --speed=17 --word=8 --parity=no"),
            ),
        ];
        for (input, karg, grub_terminal, grub_command) in cases {
            let console = Console::from_str(input).unwrap();
            assert_eq!(
                console.grub_terminal(),
                grub_terminal,
                "GRUB terminal for {}",
                input
            );
            assert_eq!(
                console.grub_command().as_deref(),
                grub_command,
                "GRUB command for {}",
                input
            );
            assert_eq!(console.karg(), karg, "karg for {}", input);
        }
    }

    #[test]
    fn invalid_console_args() {
        let cases = vec![
            "foo",
            "/dev/tty0",
            "/dev/ttyS0",
            "console=tty0",
            "console=ttyS0",
            "ztty0",
            "zttyS0",
            "tty0z",
            "ttyS0z",
            "tty1",
            "hvc1",
            "ttysclp1",
            "ttyS0,",
            "ttyS0,z",
            "ttyS0,115200p8",
            "ttyS0,115200n4",
            "ttyS0,115200n8r",
            "ttyB0",
            "ttyS9999999999999999999",
            "ttyS0,999999999999999999999",
        ];
        for input in cases {
            Console::from_str(input).unwrap_err();
        }
    }
}
