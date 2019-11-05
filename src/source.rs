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

use error_chain::bail;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::errors::*;

const DEFAULT_STREAM_BASE_URL: &str = "https://builds.coreos.fedoraproject.org/streams/";

pub trait ImageLocation: Display {
    // Obtain the image length and signature and start fetching the image
    fn source(&self) -> Result<ImageSource>;
}

// Local image source
#[derive(Debug)]
pub struct FileLocation {
    image_path: String,
    sig_path: String,
}

// Remote image source
#[derive(Debug)]
pub struct UrlLocation {
    image_url: Url,
    sig_url: Url,
    artifact_type: String,
}

// Remote image source specified by Fedora CoreOS stream metadata
#[derive(Debug)]
pub struct StreamLocation {
    stream_base_url: Option<Url>,
    stream: String,
    stream_url: Url,
    architecture: String,
    platform: String,
    format: String,
}

pub struct ImageSource {
    pub reader: Box<dyn Read>,
    pub length_hint: Option<u64>,
    pub signature: Option<Vec<u8>>,
    pub filename: String,
    pub artifact_type: String,
}

impl FileLocation {
    pub fn new(path: &str) -> Self {
        Self {
            image_path: path.to_string(),
            sig_path: format!("{}.sig", path),
        }
    }
}

impl Display for FileLocation {
    fn fmt(&self, f: &mut Formatter<'_>) -> ::std::fmt::Result {
        write!(
            f,
            "Copying image from {}\nReading signature from {}",
            self.image_path, self.sig_path
        )
    }
}

impl ImageLocation for FileLocation {
    fn source(&self) -> Result<ImageSource> {
        // open local file for reading
        let mut out = OpenOptions::new()
            .read(true)
            .open(&self.image_path)
            .chain_err(|| "opening source image file")?;

        // get size
        let length = out
            .seek(SeekFrom::End(0))
            .chain_err(|| "seeking source image file")?;
        out.seek(SeekFrom::Start(0))
            .chain_err(|| "seeking source image file")?;

        // load signature file if present
        let result = OpenOptions::new().read(true).open(&self.sig_path);
        let signature = if result.is_ok() {
            let mut sig_vec = Vec::new();
            result
                .unwrap()
                .read_to_end(&mut sig_vec)
                .chain_err(|| "reading signature file")?;
            Some(sig_vec)
        } else {
            eprintln!("Couldn't read signature file: {}", result.unwrap_err());
            None
        };
        let filename = Path::new(&self.image_path)
            .file_name()
            .chain_err(|| "extracting filename")?
            .to_string_lossy()
            .to_string();

        Ok(ImageSource {
            reader: Box::new(out),
            length_hint: Some(length),
            signature,
            filename,
            artifact_type: "disk".to_string(),
        })
    }
}

impl UrlLocation {
    pub fn new(url: &Url) -> Self {
        let mut sig_url = url.clone();
        sig_url.set_path(&format!("{}.sig", sig_url.path()));
        Self::new_with_sig_and_type(url, &sig_url, "disk")
    }

    fn new_with_sig_and_type(url: &Url, sig_url: &Url, artifact_type: &str) -> Self {
        Self {
            image_url: url.clone(),
            sig_url: sig_url.clone(),
            artifact_type: artifact_type.to_string(),
        }
    }
}

impl Display for UrlLocation {
    fn fmt(&self, f: &mut Formatter<'_>) -> ::std::fmt::Result {
        write!(
            f,
            "Downloading image from {}\nDownloading signature from {}",
            self.image_url, self.sig_url
        )
    }
}

impl ImageLocation for UrlLocation {
    fn source(&self) -> Result<ImageSource> {
        let client = reqwest::Client::new();

        // fetch signature
        let mut resp = client
            .get(self.sig_url.clone())
            .send()
            .chain_err(|| "fetching signature URL")?;
        let signature = match resp.status() {
            StatusCode::OK => {
                let mut sig_vec = Vec::new();
                resp.read_to_end(&mut sig_vec)
                    .chain_err(|| "reading signature URL")?;
                Some(sig_vec)
            }
            s => {
                eprintln!("Signature fetch failed: {}", s);
                None
            }
        };

        // start fetch, get length
        let resp = client
            .get(self.image_url.clone())
            .send()
            .chain_err(|| "fetching image URL")?;
        match resp.status() {
            StatusCode::OK => (),
            s => bail!("image fetch failed: {}", s),
        };
        let length_hint = resp.content_length();
        // ignores the Content-Disposition filename
        let filename = resp
            .url()
            .path_segments()
            .chain_err(|| "splitting image URL")?
            .next_back()
            .chain_err(|| "walking image URL")?
            .to_string();

        Ok(ImageSource {
            reader: Box::new(resp),
            length_hint,
            signature,
            filename,
            artifact_type: self.artifact_type.clone(),
        })
    }
}

impl StreamLocation {
    pub fn new(
        stream: &str,
        architecture: &str,
        platform: &str,
        format: &str,
        base_url: Option<&Url>,
    ) -> Result<Self> {
        Ok(Self {
            stream_base_url: base_url.cloned(),
            stream: stream.to_string(),
            stream_url: base_url
                .unwrap_or(&Url::parse(DEFAULT_STREAM_BASE_URL).unwrap())
                .join(&format!("{}.json", stream))
                .chain_err(|| "building stream URL")?,
            architecture: architecture.to_string(),
            platform: platform.to_string(),
            format: format.to_string(),
        })
    }
}

impl Display for StreamLocation {
    fn fmt(&self, f: &mut Formatter<'_>) -> ::std::fmt::Result {
        if self.stream_base_url.is_some() {
            write!(
                f,
                "Downloading image and signature referenced from {}",
                self.stream_url
            )
        } else {
            write!(f, "Downloading {} image and signature", self.stream)
        }
    }
}

impl ImageLocation for StreamLocation {
    fn source(&self) -> Result<ImageSource> {
        let client = reqwest::Client::new();

        // fetch stream metadata
        let resp = client
            .get(self.stream_url.clone())
            .send()
            .chain_err(|| "fetching stream metadata")?;
        match resp.status() {
            StatusCode::OK => (),
            s => bail!(
                "stream metadata fetch from {} failed: {}",
                self.stream_url,
                s
            ),
        };

        // parse it
        let stream: Stream =
            serde_json::from_reader(resp).chain_err(|| "decoding stream metadata")?;
        let artifact_type = "disk";
        let artifact = stream
            .architectures
            .get(&self.architecture)
            .map(|arch| arch.artifacts.get(&self.platform))
            .unwrap_or(None)
            .map(|platform| platform.formats.get(&self.format))
            .unwrap_or(None)
            .map(|format| format.get(artifact_type))
            .unwrap_or(None)
            .chain_err(|| {
                format!(
                    "couldn't find architecture {}, platform {}, format {} disk image in stream metadata",
                    self.architecture,
                    self.platform,
                    self.format
                )
            })?;

        // let UrlLocation handle the rest
        UrlLocation::new_with_sig_and_type(
            &Url::parse(&artifact.location)
                .chain_err(|| "parsing image URL from stream metadata")?,
            &Url::parse(&artifact.signature)
                .chain_err(|| "parsing signature URL from stream metadata")?,
            &artifact_type,
        )
        .source()
    }
}

#[derive(Debug, Deserialize)]
struct Stream {
    architectures: HashMap<String, Arch>,
}

#[derive(Debug, Deserialize)]
struct Arch {
    artifacts: HashMap<String, Platform>,
}

#[derive(Debug, Deserialize)]
struct Platform {
    formats: HashMap<String, HashMap<String, Artifact>>,
}

#[derive(Debug, Deserialize)]
struct Artifact {
    location: String,
    signature: String,
}
