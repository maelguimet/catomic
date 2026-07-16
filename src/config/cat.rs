//! Purpose: configure small cat-themed presentation and bounded recovery policy.
//! Owns: the default-on status badge plus opt-in `.catnap` limits and TOML loading.
//! Must not: mutate editor state, write files, inspect buffers, or start background work.
//! Invariants: recovery is disabled by default and every enabled workload is bounded.
//! Phase: 8 cat polish and recovery.

use std::io;
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CatConfig {
    pub(crate) status_messages: bool,
    pub(crate) recovery: RecoveryConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecoveryConfig {
    pub(crate) enabled: bool,
    pub(crate) interval_secs: u64,
    pub(crate) max_bytes: usize,
}

impl Default for CatConfig {
    fn default() -> Self {
        Self {
            status_messages: true,
            recovery: RecoveryConfig::default(),
        }
    }
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 30,
            max_bytes: 1024 * 1024,
        }
    }
}

pub(crate) fn parse(text: &str) -> io::Result<CatConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        cat: CatConfigFile,
        #[serde(default)]
        recovery: RecoveryConfigFile,
    }

    #[derive(Deserialize)]
    #[serde(default)]
    struct RecoveryConfigFile {
        enabled: bool,
        interval_secs: u64,
        max_bytes: usize,
    }

    #[derive(Deserialize)]
    #[serde(default)]
    struct CatConfigFile {
        status_messages: bool,
    }

    impl Default for CatConfigFile {
        fn default() -> Self {
            Self {
                status_messages: true,
            }
        }
    }

    impl Default for RecoveryConfigFile {
        fn default() -> Self {
            let defaults = RecoveryConfig::default();
            Self {
                enabled: defaults.enabled,
                interval_secs: defaults.interval_secs,
                max_bytes: defaults.max_bytes,
            }
        }
    }

    let config = super::decode::<ConfigFile>(text)?;
    validate_recovery(config.recovery.interval_secs, config.recovery.max_bytes)?;
    Ok(CatConfig {
        status_messages: config.cat.status_messages,
        recovery: RecoveryConfig {
            enabled: config.recovery.enabled,
            interval_secs: config.recovery.interval_secs,
            max_bytes: config.recovery.max_bytes,
        },
    })
}

fn validate_recovery(interval_secs: u64, max_bytes: usize) -> io::Result<()> {
    if !(5..=3_600).contains(&interval_secs) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery.interval_secs must be between 5 and 3600",
        ));
    }
    if !(1..=16 * 1024 * 1024).contains(&max_bytes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery.max_bytes must be between 1 and 16777216",
        ));
    }
    Ok(())
}

pub(crate) fn load_from(path: &Path) -> io::Result<CatConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(CatConfig::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<CatConfig> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    match super::big_files::config_path(xdg.as_deref(), home.as_deref()) {
        Some(path) => load_from(&path),
        None => Ok(CatConfig::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_a_tasteful_status_badge() {
        assert_eq!(parse("").unwrap(), CatConfig::default());
        assert!(CatConfig::default().status_messages);
    }

    #[test]
    fn status_messages_can_be_disabled_and_require_a_boolean() {
        assert!(
            !parse("[cat]\nstatus_messages = false\n")
                .unwrap()
                .status_messages
        );
        assert!(parse("[cat]\nstatus_messages = \"no\"\n").is_err());
    }

    #[test]
    fn recovery_is_opt_in_and_bounded() {
        let defaults = parse("").unwrap().recovery;
        assert_eq!(defaults, RecoveryConfig::default());
        assert!(!defaults.enabled);

        let configured =
            parse("[recovery]\nenabled = true\ninterval_secs = 10\nmax_bytes = 4096\n")
                .unwrap()
                .recovery;
        assert!(configured.enabled);
        assert_eq!(configured.interval_secs, 10);
        assert_eq!(configured.max_bytes, 4096);
    }

    #[test]
    fn recovery_rejects_unbounded_values() {
        for text in [
            "[recovery]\ninterval_secs = 4\n",
            "[recovery]\ninterval_secs = 3601\n",
            "[recovery]\nmax_bytes = 0\n",
            "[recovery]\nmax_bytes = 16777217\n",
        ] {
            assert!(parse(text).is_err(), "config should fail: {text}");
        }
    }
}
