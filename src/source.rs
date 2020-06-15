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
use reqwest::{blocking, StatusCode, Url};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cmdline::*;
use crate::errors::*;
use crate::osmet::*;

/// Completion timeout for HTTP requests (4 hours).
const HTTP_COMPLETION_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);

/// Default base URL to Fedora CoreOS streams metadata.
const DEFAULT_STREAM_BASE_URL: &str = "https://builds.coreos.fedoraproject.org/streams/";

/// Directory in which we look for osmet files.
const OSMET_FILES_DIR: &str = "/run/coreos-installer/osmet";

pub trait ImageLocation: Display {
    // Obtain image lengths and signatures and start fetching the images
    fn sources(&self) -> Result<Vec<ImageSource>>;

    // Whether GPG signature verification is required by default
    fn require_signature(&self) -> bool {
        true
    }
}

// Local image source
#[derive(Debug)]
pub struct FileLocation {
    image_path: String,
    sig_path: String,
}

// Local osmet image source
pub struct OsmetLocation {
    osmet_path: PathBuf,
    architecture: String,
    sector_size: u32,
    description: String,
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
    fn sources(&self) -> Result<Vec<ImageSource>> {
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
        let signature = match OpenOptions::new().read(true).open(&self.sig_path) {
            Ok(mut file) => {
                let mut sig_vec = Vec::new();
                file.read_to_end(&mut sig_vec)
                    .chain_err(|| "reading signature file")?;
                Some(sig_vec)
            }
            Err(err) => {
                eprintln!("Couldn't read signature file: {}", err);
                None
            }
        };
        let filename = Path::new(&self.image_path)
            .file_name()
            .chain_err(|| "extracting filename")?
            .to_string_lossy()
            .to_string();

        Ok(vec![ImageSource {
            reader: Box::new(out),
            length_hint: Some(length),
            signature,
            filename,
            artifact_type: "disk".to_string(),
        }])
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

    /// Fetch signature content from URL.
    fn fetch_signature(sig_url: &Url) -> Result<Vec<u8>> {
        let client = new_http_client()?;
        let mut resp = client
            .get(sig_url.clone())
            .send()
            .chain_err(|| "sending signature request")?
            .error_for_status()
            .chain_err(|| "fetching signature URL")?;

        let mut sig_bytes = Vec::new();
        resp.read_to_end(&mut sig_bytes)
            .chain_err(|| "reading signature content")?;
        Ok(sig_bytes)
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
    fn sources(&self) -> Result<Vec<ImageSource>> {
        let signature = Self::fetch_signature(&self.sig_url)
            .map_err(|e| eprintln!("Failed to fetch signature: {}", e))
            .ok();

        // start fetch, get length
        let client = new_http_client()?;
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

        Ok(vec![ImageSource {
            reader: Box::new(resp),
            length_hint,
            signature,
            filename,
            artifact_type: self.artifact_type.clone(),
        }])
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
            stream_url: build_stream_url(stream, base_url)?,
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
                "Downloading image ({}) and signature referenced from {}",
                self.format, self.stream_url
            )
        } else {
            write!(
                f,
                "Downloading {} image ({}) and signature",
                self.stream, self.format
            )
        }
    }
}

impl ImageLocation for StreamLocation {
    fn sources(&self) -> Result<Vec<ImageSource>> {
        // fetch and parse stream metadata
        let client = new_http_client()?;
        let stream = fetch_stream(client, &self.stream_url)?;

        // descend it
        let artifacts = stream
            .architectures
            .get(&self.architecture)
            .map(|arch| arch.artifacts.get(&self.platform))
            .unwrap_or(None)
            .map(|platform| platform.formats.get(&self.format))
            .unwrap_or(None)
            .chain_err(|| {
                format!(
                    "couldn't find architecture {}, platform {}, format {} in stream metadata",
                    self.architecture, self.platform, self.format
                )
            })?;

        // build sources, letting UrlLocation handle the details
        let mut sources: Vec<ImageSource> = Vec::new();
        for (artifact_type, artifact) in artifacts.iter() {
            let artifact_url = Url::parse(&artifact.location)
                .chain_err(|| "parsing artifact URL from stream metadata")?;
            let signature_url = Url::parse(&artifact.signature)
                .chain_err(|| "parsing signature URL from stream metadata")?;
            let mut artifact_sources =
                UrlLocation::new_with_sig_and_type(&artifact_url, &signature_url, &artifact_type)
                    .sources()?;
            sources.append(&mut artifact_sources);
        }
        sources.sort_by_key(|k| k.artifact_type.to_string());
        Ok(sources)
    }
}

impl OsmetLocation {
    pub fn new(architecture: &str, sector_size: u32) -> Result<Option<Self>> {
        let osmet_dir = Path::new(OSMET_FILES_DIR);
        if !osmet_dir.exists() {
            return Ok(None);
        }

        if let Some((osmet_path, description)) =
            find_matching_osmet_in_dir(osmet_dir, architecture, sector_size)?
        {
            Ok(Some(Self {
                osmet_path,
                architecture: architecture.into(),
                sector_size,
                description,
            }))
        } else {
            Ok(None)
        }
    }
}

impl Display for OsmetLocation {
    fn fmt(&self, f: &mut Formatter<'_>) -> ::std::fmt::Result {
        write!(
            f,
            "Installing {} {} ({}-byte sectors)",
            self.description, self.architecture, self.sector_size
        )
    }
}

impl ImageLocation for OsmetLocation {
    fn sources(&self) -> Result<Vec<ImageSource>> {
        let unpacker = OsmetUnpacker::new_from_sysroot(Path::new(&self.osmet_path))?;

        let filename = {
            let stem = self.osmet_path.file_stem().ok_or_else(|| {
                // This really should never happen since for us to get here, we must've found a
                // valid osmet file... But let's still just error out instead of assert in case
                // somehow this doesn't hold true in the future and a user hits this.
                format!(
                    "can't create new .raw filename from osmet path {:?}",
                    &self.osmet_path
                )
            })?;
            // really we don't need to care about UTF-8 here, but ImageSource right now does
            let mut filename: String = stem
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 osmet file stem: {:?}", stem))?
                .into();
            filename.push_str(".raw");
            filename
        };
        let length = unpacker.length();
        Ok(vec![ImageSource {
            reader: Box::new(unpacker),
            length_hint: Some(length),
            signature: None,
            filename,
            artifact_type: "disk".to_string(),
        }])
    }

    // For osmet, we don't require GPG verification since we trust osmet files placed in the
    // OSMET_FILES_DIR.
    fn require_signature(&self) -> bool {
        false
    }
}

/// Subcommand to list objects available in stream metadata.
pub fn list_stream(config: &ListStreamConfig) -> Result<()> {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Row<'a> {
        architecture: &'a str,
        platform: &'a str,
        format: &'a str,
    }

    // fetch stream metadata
    let client = new_http_client()?;
    let stream_url = build_stream_url(&config.stream, config.stream_base_url.as_ref())?;
    let stream = fetch_stream(client, &stream_url)?;

    // walk formats
    let mut rows: Vec<Row> = Vec::new();
    for (architecture_name, architecture) in stream.architectures.iter() {
        for (platform_name, platform) in architecture.artifacts.iter() {
            for format_name in platform.formats.keys() {
                rows.push(Row {
                    architecture: architecture_name,
                    platform: platform_name,
                    format: format_name,
                });
            }
        }
    }
    rows.sort();

    // add header row
    rows.insert(
        0,
        Row {
            architecture: "Architecture",
            platform: "Platform",
            format: "Format",
        },
    );

    // calculate field widths
    let mut widths: [usize; 2] = [0; 2];
    for row in &rows {
        widths[0] = widths[0].max(row.architecture.len());
        widths[1] = widths[1].max(row.platform.len());
    }

    // report results
    for row in &rows {
        println!(
            "{:3$}  {:4$}  {}",
            row.architecture, row.platform, row.format, widths[0], widths[1]
        );
    }
    Ok(())
}

/// Generate a stream URL from a stream name and base URL, or the default
/// base URL if none is specified.
fn build_stream_url(stream: &str, base_url: Option<&Url>) -> Result<Url> {
    Ok(base_url
        .unwrap_or(&Url::parse(DEFAULT_STREAM_BASE_URL).unwrap())
        .join(&format!("{}.json", stream))
        .chain_err(|| "building stream URL")?)
}

/// Fetch and parse stream metadata.
fn fetch_stream(client: blocking::Client, url: &Url) -> Result<Stream> {
    // fetch stream metadata
    let resp = client
        .get(url.clone())
        .send()
        .chain_err(|| "fetching stream metadata")?;
    match resp.status() {
        StatusCode::OK => (),
        s => bail!("stream metadata fetch from {} failed: {}", url, s),
    };

    // parse it
    let stream: Stream = serde_json::from_reader(resp).chain_err(|| "decoding stream metadata")?;
    Ok(stream)
}

/// Customize and build a new HTTP client.
pub fn new_http_client() -> Result<blocking::Client> {
    blocking::ClientBuilder::new()
        .timeout(HTTP_COMPLETION_TIMEOUT)
        .build()
        .chain_err(|| "building HTTP client")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_http_client() {
        let _ = new_http_client().unwrap();
    }
}
