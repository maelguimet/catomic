//! Purpose: fetch official release metadata and assets under a strict network policy.
//! Owns: trusted origins, redirects, timeouts, response bounds, and exact asset sizes.
//! Must not: use ambient proxies, accept arbitrary URLs, persist downloads, or install bytes.
//! Invariants: production requests are HTTPS-only and redirects remain on allowlisted hosts.
//! Phase: safe self-update workflow.

use std::net::IpAddr;
use std::time::Duration;

use serde::Deserialize;

use super::security::ReleaseVersion;
use super::{
    UpdateError, CONNECT_TIMEOUT, EXIT_NETWORK, EXIT_UNSUPPORTED, MAX_ARTIFACT_BYTES,
    MAX_CHECKSUM_BYTES, MAX_METADATA_BYTES, REQUEST_TIMEOUT,
};

#[derive(Debug)]
pub(super) struct ReleaseInfo {
    pub(super) version: ReleaseVersion,
    pub(super) binary: Asset,
    pub(super) checksum: Asset,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Asset {
    pub(super) name: String,
    #[serde(rename = "browser_download_url")]
    pub(super) url: String,
    pub(super) size: u64,
}

pub(super) struct HttpClient {
    client: reqwest::Client,
    api_url: String,
    allow_loopback: bool,
}

impl HttpClient {
    pub(super) fn new(api_url: &str) -> Result<Self, UpdateError> {
        Self::build(api_url, false, REQUEST_TIMEOUT)
    }

    pub(super) fn build(
        api_url: &str,
        allow_loopback: bool,
        timeout: Duration,
    ) -> Result<Self, UpdateError> {
        let parsed = reqwest::Url::parse(api_url).map_err(|error| {
            UpdateError::new(EXIT_NETWORK, format!("invalid update URL: {error}"))
        })?;
        if !allowed_url(&parsed, allow_loopback) {
            return Err(UpdateError::new(
                EXIT_NETWORK,
                format!("refusing update URL {api_url}"),
            ));
        }
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("too many update redirects");
            }
            if allowed_url(attempt.url(), allow_loopback) {
                attempt.follow()
            } else {
                attempt.error("update redirect left the trusted origin policy")
            }
        });
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(CONNECT_TIMEOUT.min(timeout))
            .redirect(policy)
            .no_proxy()
            .user_agent(concat!("catomic/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                UpdateError::new(EXIT_NETWORK, format!("create update client: {error}"))
            })?;
        Ok(Self {
            client,
            api_url: api_url.to_string(),
            allow_loopback,
        })
    }

    pub(super) async fn latest(&self, asset_name: &str) -> Result<ReleaseInfo, UpdateError> {
        let bytes = self
            .get_bounded(&self.api_url, MAX_METADATA_BYTES, None)
            .await?;
        #[derive(Deserialize)]
        struct Release {
            tag_name: String,
            assets: Vec<Asset>,
        }
        let release: Release = serde_json::from_slice(&bytes).map_err(|error| {
            UpdateError::new(
                EXIT_NETWORK,
                format!("invalid GitHub release metadata: {error}"),
            )
        })?;
        let version_text = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);
        let version = ReleaseVersion::parse(version_text).map_err(|error| {
            UpdateError::new(
                EXIT_NETWORK,
                format!("invalid release tag {:?}: {error}", release.tag_name),
            )
        })?;
        let checksum_name = format!("{asset_name}.sha256");
        let binary = find_asset(
            &release.assets,
            asset_name,
            MAX_ARTIFACT_BYTES,
            self.allow_loopback,
        )?;
        let checksum = find_asset(
            &release.assets,
            &checksum_name,
            MAX_CHECKSUM_BYTES,
            self.allow_loopback,
        )?;
        Ok(ReleaseInfo {
            version,
            binary,
            checksum,
        })
    }

    pub(super) async fn download_release(
        &self,
        release: &ReleaseInfo,
    ) -> Result<(Vec<u8>, Vec<u8>), UpdateError> {
        let checksum = self
            .get_bounded(
                &release.checksum.url,
                MAX_CHECKSUM_BYTES,
                Some(release.checksum.size),
            )
            .await?;
        let binary = self
            .get_bounded(
                &release.binary.url,
                MAX_ARTIFACT_BYTES,
                Some(release.binary.size),
            )
            .await?;
        Ok((checksum, binary))
    }

    pub(super) async fn get_bounded(
        &self,
        url: &str,
        limit: usize,
        expected_size: Option<u64>,
    ) -> Result<Vec<u8>, UpdateError> {
        let parsed = reqwest::Url::parse(url).map_err(|error| {
            UpdateError::new(EXIT_NETWORK, format!("invalid update URL: {error}"))
        })?;
        if !allowed_url(&parsed, self.allow_loopback) {
            return Err(UpdateError::new(
                EXIT_NETWORK,
                format!("refusing untrusted update URL {url}"),
            ));
        }
        if expected_size.is_some_and(|size| size > limit as u64) {
            return Err(UpdateError::new(
                EXIT_NETWORK,
                format!("update asset declares more than {limit} bytes"),
            ));
        }
        let mut response = self.client.get(parsed).send().await.map_err(|error| {
            UpdateError::new(EXIT_NETWORK, format!("update request failed: {error}"))
        })?;
        if !response.status().is_success() {
            return Err(UpdateError::new(
                EXIT_NETWORK,
                format!("update server returned HTTP {}", response.status()),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(UpdateError::new(
                EXIT_NETWORK,
                format!("update response exceeded {limit} bytes"),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            UpdateError::new(EXIT_NETWORK, format!("read update response: {error}"))
        })? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(UpdateError::new(
                    EXIT_NETWORK,
                    format!("update response exceeded {limit} bytes"),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if expected_size.is_some_and(|size| size != bytes.len() as u64) {
            return Err(UpdateError::new(
                EXIT_NETWORK,
                format!(
                    "update asset size mismatch: expected {} bytes, received {}",
                    expected_size.unwrap_or_default(),
                    bytes.len()
                ),
            ));
        }
        Ok(bytes)
    }
}

fn find_asset(
    assets: &[Asset],
    name: &str,
    limit: usize,
    allow_loopback: bool,
) -> Result<Asset, UpdateError> {
    let asset = assets
        .iter()
        .find(|asset| asset.name == name)
        .cloned()
        .ok_or_else(|| {
            UpdateError::new(EXIT_UNSUPPORTED, format!("release has no {name} asset"))
        })?;
    if asset.size > limit as u64 {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            format!("release asset {name} exceeds the {limit}-byte limit"),
        ));
    }
    let url = reqwest::Url::parse(&asset.url)
        .map_err(|error| UpdateError::new(EXIT_NETWORK, format!("invalid asset URL: {error}")))?;
    if !allowed_url(&url, allow_loopback) {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            format!("release asset {name} uses an untrusted URL"),
        ));
    }
    if !allow_loopback
        && !asset
            .url
            .starts_with("https://github.com/maelguimet/catomic/releases/download/")
    {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            format!("release asset {name} is outside the official release path"),
        ));
    }
    Ok(asset)
}

fn allowed_url(url: &reqwest::Url, allow_loopback: bool) -> bool {
    if allow_loopback && url.scheme() == "http" && url.host_str().is_some_and(is_loopback) {
        return true;
    }
    if url.scheme() != "https" {
        return false;
    }
    matches!(
        url.host_str(),
        Some(
            "api.github.com"
                | "github.com"
                | "raw.githubusercontent.com"
                | "objects.githubusercontent.com"
                | "release-assets.githubusercontent.com"
        )
    )
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(|character| character == '[' || character == ']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}
