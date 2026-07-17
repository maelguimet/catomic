//! Purpose: validate managed-release versions, revisions, and SHA-256 checksum files.
//! Owns: SemVer precedence, strict checksum grammar, digest comparison, and SHA syntax.
//! Must not: contact a network, read files, execute candidates, or choose release assets.
//! Invariants: downgrades compare correctly and checksums bind one exact asset filename.
//! Phase: safe self-update workflow.

use std::cmp::Ordering;
use std::fmt;

use ring::digest::{digest, SHA256};

use super::{UpdateError, EXIT_NETWORK};

pub(super) fn verify_checksum(
    binary: &[u8],
    checksum: &[u8],
    filename: &str,
) -> Result<(), UpdateError> {
    let text = std::str::from_utf8(checksum)
        .map_err(|_| UpdateError::new(EXIT_NETWORK, "checksum file is not valid UTF-8"))?;
    let mut fields = text.split_whitespace();
    let expected = fields.next().filter(|hash| valid_hash(hash));
    let named = fields.next().map(|name| name.trim_start_matches('*'));
    if expected.is_none() || named != Some(filename) || fields.next().is_some() {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            "checksum file must contain exactly one SHA-256 entry for the selected asset",
        ));
    }
    let actual: String = digest(&SHA256, binary)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if expected.is_some_and(|expected| actual.eq_ignore_ascii_case(expected)) {
        Ok(())
    } else {
        Err(UpdateError::new(
            EXIT_NETWORK,
            "downloaded binary failed SHA-256 verification",
        ))
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Debug)]
pub(super) struct ReleaseVersion {
    raw: String,
    core: [u64; 3],
    prerelease: Vec<Identifier>,
}

impl PartialEq for ReleaseVersion {
    fn eq(&self, other: &Self) -> bool {
        self.core == other.core && self.prerelease == other.prerelease
    }
}

impl Eq for ReleaseVersion {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Identifier {
    Numeric(u64),
    Text(String),
}

impl ReleaseVersion {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        let without_build = value.split_once('+').map_or(value, |(version, _)| version);
        let (core, prerelease) = without_build
            .split_once('-')
            .map_or((without_build, None), |(core, prerelease)| {
                (core, Some(prerelease))
            });
        let numbers: Vec<u64> = core
            .split('.')
            .map(|part| {
                part.parse::<u64>()
                    .map_err(|_| format!("invalid numeric version component {part:?}"))
            })
            .collect::<Result<_, _>>()?;
        let core: [u64; 3] = numbers
            .try_into()
            .map_err(|_| "version must contain major.minor.patch".to_string())?;
        let prerelease = prerelease
            .map(|text| {
                if text.is_empty() {
                    return Err("pre-release version must not be empty".to_string());
                }
                text.split('.')
                    .map(|part| {
                        if part.is_empty()
                            || !part
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                        {
                            return Err(format!("invalid pre-release component {part:?}"));
                        }
                        match part.parse::<u64>() {
                            Ok(number) if part == "0" || !part.starts_with('0') => {
                                Ok(Identifier::Numeric(number))
                            }
                            Ok(_) => Err(format!(
                                "numeric pre-release component has a leading zero: {part:?}"
                            )),
                            Err(_) => Ok(Identifier::Text(part.to_string())),
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            raw: value.to_string(),
            core,
            prerelease,
        })
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.raw)
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => compare_prerelease(&self.prerelease, &other.prerelease),
            }
        })
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_prerelease(left: &[Identifier], right: &[Identifier]) -> Ordering {
    for (left, right) in left.iter().zip(right) {
        let ordering = match (left, right) {
            (Identifier::Numeric(left), Identifier::Numeric(right)) => left.cmp(right),
            (Identifier::Numeric(_), Identifier::Text(_)) => Ordering::Less,
            (Identifier::Text(_), Identifier::Numeric(_)) => Ordering::Greater,
            (Identifier::Text(left), Identifier::Text(right)) => left.cmp(right),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}
