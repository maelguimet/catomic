//! Purpose: configure the small, optional cat-themed presentation touches.
//! Owns: the default-on status badge flag and typed TOML loading.
//! Must not: mutate editor state, write config, affect file safety, or start work.
//! Invariants: missing config enables the tasteful badge; only a boolean is accepted.
//! Phase: 8 cat polish.

use std::io;
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CatConfig {
    pub(crate) status_messages: bool,
}

impl Default for CatConfig {
    fn default() -> Self {
        Self {
            status_messages: true,
        }
    }
}

pub(crate) fn parse(text: &str) -> io::Result<CatConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        cat: CatConfigFile,
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

    let config = super::decode::<ConfigFile>(text)?.cat;
    Ok(CatConfig {
        status_messages: config.status_messages,
    })
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
}
