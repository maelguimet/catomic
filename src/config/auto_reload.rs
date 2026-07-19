//! Purpose: configure automatic reload of clean buffers after external edits.
//! Owns: the default-on flag and typed TOML decoding.
//! Must not: construct watchers, open editor buffers, write config, or know UI.
//! Invariants: missing config enables auto reload; only
//!   `[files] auto_reload = true|false` affects this setting.
//! Phase: 2-bx automatic external reload policy.

use std::io;
#[cfg(test)]
use std::path::Path;

use serde::Deserialize;

pub(crate) const DEFAULT_AUTO_RELOAD: bool = true;

pub(crate) fn parse(text: &str) -> io::Result<bool> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        files: FileSettings,
    }

    #[derive(Deserialize)]
    #[serde(default)]
    struct FileSettings {
        auto_reload: bool,
    }

    impl Default for FileSettings {
        fn default() -> Self {
            Self {
                auto_reload: DEFAULT_AUTO_RELOAD,
            }
        }
    }

    Ok(super::decode::<ConfigFile>(text)?.files.auto_reload)
}

#[cfg(test)]
fn load_from(path: &Path) -> io::Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DEFAULT_AUTO_RELOAD),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_enabled_when_missing() {
        let path = std::env::temp_dir().join(format!(
            "catomic_missing_auto_reload_{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        assert!(load_from(&path).unwrap());
    }

    #[test]
    fn parses_enabled_and_disabled_inside_files_section() {
        assert!(parse("[files]\nauto_reload = true\n").unwrap());
        assert!(!parse("[files]\nauto_reload = false\n").unwrap());
    }

    #[test]
    fn ignores_other_sections_and_rejects_invalid_values() {
        assert!(parse("[other]\nauto_reload = false\n").unwrap());
        let error = parse("[files]\nauto_reload = sometimes\n").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("auto_reload"));
    }
}
