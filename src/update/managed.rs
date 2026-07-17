//! Purpose: coordinate updates for official Catomic release binaries.
//! Owns: managed-update reporting, confirmation, candidate validation, and install handoff.
//! Must not: define transport policy, parse checksums, accept downgrades, or touch config.
//! Invariants: verified candidates pass version/config checks before atomic replacement.
//! Phase: safe self-update workflow.

mod http;
mod security;

#[cfg(test)]
mod tests;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::Deserialize;

use crate::cli::UpdateOptions;

use self::http::{HttpClient, ReleaseInfo};
use self::security::{valid_sha, verify_checksum, ReleaseVersion};
use super::{
    confirm, maybe_backup, process, UpdateError, EXIT_CONFIG, EXIT_INSTALL, EXIT_NETWORK,
    EXIT_UNSUPPORTED,
};

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/maelguimet/catomic/releases/latest";
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_CHECKSUM_BYTES: usize = 64 * 1024;
const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) const fn is_managed_build() -> bool {
    matches!(option_env!("CATOMIC_MANAGED_RELEASE"), Some("1"))
}

pub(super) fn run(options: UpdateOptions) -> Result<(), UpdateError> {
    let asset_name = asset_name()?;
    println!("install method: managed release binary");
    println!("source: {LATEST_RELEASE_URL}");
    println!("current version: {}", env!("CARGO_PKG_VERSION"));
    let client = HttpClient::new(LATEST_RELEASE_URL)?;
    if options.check {
        let release = block_on(client.latest(asset_name))?;
        print_check(&release);
        println!("writes performed: none");
        return Ok(());
    }
    if !confirm(
        options,
        "Check and install an official checksummed release? Network and disk writes will follow.",
    )? {
        println!("update cancelled; no network or disk changes made");
        return Ok(());
    }
    let release = block_on(client.latest(asset_name))?;
    let current = current_version()?;
    if release.version <= current {
        println!("available version: {}", release.version);
        println!("already up to date; no files changed");
        return Ok(());
    }
    println!("available version: {}", release.version);
    println!("artifact: {}", release.binary.url);
    let backup = maybe_backup(options)?;
    let (checksum, binary) = block_on(client.download_release(&release))?;
    verify_checksum(&binary, &checksum, &release.binary.name)?;
    validate_candidate(&binary, &release.version.to_string())?;
    let receipt = super::install::replace_current(&binary, env!("CARGO_PKG_VERSION"))
        .map_err(|error| UpdateError::new(EXIT_INSTALL, error))?;
    println!("old version: {}", env!("CARGO_PKG_VERSION"));
    println!("new version: {}", release.version);
    println!("user state: unchanged");
    match backup {
        Some(path) => println!("user-state backup: {}", path.display()),
        None => println!("user-state backup: not requested"),
    }
    println!("rollback binary: {}", receipt.rollback_path().display());
    println!(
        "rollback command: cp -- {} {}",
        shell_quote(receipt.rollback_path()),
        shell_quote(&std::env::current_exe().unwrap_or_default())
    );
    Ok(())
}

pub(super) fn source_version_at(sha: &str) -> Result<String, UpdateError> {
    if !valid_sha(sha) {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            "refusing invalid source revision",
        ));
    }
    let url = format!(
        "https://raw.githubusercontent.com/maelguimet/catomic/{sha}/Cargo.toml"
    );
    let client = HttpClient::new(&url)?;
    let bytes = block_on(client.get_bounded(&url, 64 * 1024, None))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        UpdateError::new(EXIT_NETWORK, "official Cargo.toml is not valid UTF-8")
    })?;
    let manifest: toml::Value = toml::from_str(text).map_err(|error| {
        UpdateError::new(
            EXIT_NETWORK,
            format!("official Cargo.toml is invalid: {error}"),
        )
    })?;
    manifest
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            UpdateError::new(
                EXIT_NETWORK,
                "official Cargo.toml has no package version",
            )
        })
}

pub(super) fn source_relation(base: &str, head: &str) -> Result<String, UpdateError> {
    if !valid_sha(base) || !valid_sha(head) {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            "refusing invalid source revision",
        ));
    }
    if base == head {
        return Ok("identical".to_string());
    }
    let url = format!(
        "https://api.github.com/repos/maelguimet/catomic/compare/{base}...{head}"
    );
    let client = HttpClient::new(&url)?;
    let bytes = block_on(client.get_bounded(&url, MAX_METADATA_BYTES, None))?;
    #[derive(Deserialize)]
    struct Comparison {
        status: String,
    }
    let comparison: Comparison = serde_json::from_slice(&bytes).map_err(|error| {
        UpdateError::new(
            EXIT_NETWORK,
            format!("invalid GitHub comparison response: {error}"),
        )
    })?;
    match comparison.status.as_str() {
        "behind" | "identical" | "ahead" | "diverged" => Ok(comparison.status),
        status => Err(UpdateError::new(
            EXIT_NETWORK,
            format!("GitHub returned unknown comparison status {status:?}"),
        )),
    }
}

pub(super) fn source_version_is_downgrade(version: &str) -> Result<bool, UpdateError> {
    let remote = ReleaseVersion::parse(version).map_err(|error| {
        UpdateError::new(
            EXIT_NETWORK,
            format!("official source version {version:?} is invalid: {error}"),
        )
    })?;
    Ok(remote < current_version()?)
}

fn print_check(release: &ReleaseInfo) {
    let current = current_version();
    println!("available version: {}", release.version);
    match current {
        Ok(current) => {
            println!(
                "update available: {}",
                if release.version > current { "yes" } else { "no" }
            );
            println!(
                "can apply: {}",
                if release.version >= current { "yes" } else { "no" }
            );
            if release.version < current {
                println!("reason: latest release is older; downgrades are refused");
            }
        }
        Err(error) => println!("can apply: no ({error})"),
    }
    println!("artifact: {}", release.binary.url);
    println!("verification: SHA-256 ({})", release.checksum.url);
}

fn current_version() -> Result<ReleaseVersion, UpdateError> {
    ReleaseVersion::parse(env!("CARGO_PKG_VERSION")).map_err(|error| {
        UpdateError::new(
            EXIT_UNSUPPORTED,
            format!("current package version is invalid: {error}"),
        )
    })
}

fn asset_name() -> Result<&'static str, UpdateError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("catomic-x86_64-unknown-linux-gnu"),
        arch => Err(UpdateError::new(
            EXIT_UNSUPPORTED,
            format!("managed updates are not published for architecture {arch}"),
        )),
    }
}

fn block_on<T>(
    future: impl std::future::Future<Output = Result<T, UpdateError>>,
) -> Result<T, UpdateError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            UpdateError::new(EXIT_NETWORK, format!("create network runtime: {error}"))
        })?;
    runtime.block_on(future)
}

fn validate_candidate(bytes: &[u8], expected_version: &str) -> Result<(), UpdateError> {
    let candidate = TempBinary::create(bytes)?;
    let mut config = Command::new(&candidate.path);
    config.args(["update", "--validate-config"]);
    process::run_checked(&mut config, REQUEST_TIMEOUT, MAX_METADATA_BYTES).map_err(|error| {
        UpdateError::new(
            EXIT_CONFIG,
            format!("new version rejected the existing configuration: {error}"),
        )
    })?;
    let mut version = Command::new(&candidate.path);
    version.arg("--version");
    let output = process::run_checked(&mut version, REQUEST_TIMEOUT, 16 * 1024)
        .map_err(|error| UpdateError::new(EXIT_INSTALL, error))?;
    let reported = String::from_utf8_lossy(&output.stdout);
    if reported.trim() != format!("catomic {expected_version}") {
        return Err(UpdateError::new(
            EXIT_INSTALL,
            format!(
                "downloaded binary reports {:?}, expected {expected_version:?}",
                reported.trim()
            ),
        ));
    }
    Ok(())
}

struct TempBinary {
    path: PathBuf,
}

impl TempBinary {
    fn create(bytes: &[u8]) -> Result<Self, UpdateError> {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "catomic-update-candidate-{}-{suffix}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o700);
        let mut file = options.open(&path).map_err(|error| {
            UpdateError::new(
                EXIT_INSTALL,
                format!("create candidate validation file: {error}"),
            )
        })?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|error| {
                UpdateError::new(
                    EXIT_INSTALL,
                    format!("set candidate permissions: {error}"),
                )
            })?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                UpdateError::new(EXIT_INSTALL, format!("write candidate binary: {error}"))
            })?;
        Ok(Self { path })
    }
}

impl Drop for TempBinary {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir())
            && self.path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("catomic-update-candidate-")
            })
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
