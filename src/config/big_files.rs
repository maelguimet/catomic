//! Purpose: configure bounded line pages for oversized file viewing.
//! Owns: page-line default, typed TOML decoding, and config path loading.
//! Must not: open editor buffers, scan user files, write config, or know App UI.
//! Invariants: page_lines is nonzero; missing config uses defaults; config roots
//!   must be absolute; only `[big_files] page_lines = N` affects this configuration.
//! Phase: 2-bk configurable paged-file policy.

use std::io;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use serde::Deserialize;

pub(crate) const DEFAULT_PAGE_LINES: usize = 20_000;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct BigFileConfig {
    pub(crate) page_lines: usize,
}

impl Default for BigFileConfig {
    fn default() -> Self {
        Self {
            page_lines: DEFAULT_PAGE_LINES,
        }
    }
}

pub(crate) fn parse(text: &str) -> io::Result<BigFileConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        big_files: BigFileConfig,
    }

    let config = super::decode::<ConfigFile>(text)?.big_files;
    if config.page_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "big_files.page_lines must be a positive integer",
        ));
    }
    Ok(config)
}

pub(crate) fn load_from(path: &Path) -> io::Result<BigFileConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BigFileConfig::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<BigFileConfig> {
    match super::user_file::optional_path() {
        Some(path) => load_from(&path),
        None => Ok(BigFileConfig::default()),
    }
}

#[cfg(test)]
fn config_path(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    super::user_file::resolve_path(xdg_config_home, home).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_uses_default_page_lines() {
        let path = std::env::temp_dir().join(format!(
            "catomic_missing_big_config_{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        assert_eq!(load_from(&path).unwrap().page_lines, DEFAULT_PAGE_LINES);
    }

    #[test]
    fn parses_big_file_page_lines_and_ignores_other_settings() {
        let config = parse(
            "# catomic\n[editor]\ntab_size = 4\n\n[big_files]\npage_lines = 200\nfuture = true\n",
        )
        .unwrap();

        assert_eq!(config.page_lines, 200);
    }

    #[test]
    fn accepts_standard_toml_comments_and_numeric_separators() {
        let config = parse("[big_files]\npage_lines = 20_000 # one page\n").unwrap();

        assert_eq!(config.page_lines, 20_000);
    }

    #[test]
    fn rejects_zero_and_non_numeric_page_lines() {
        for text in [
            "[big_files]\npage_lines = 0\n",
            "[big_files]\npage_lines = many\n",
        ] {
            let error = parse(text).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("page_lines"));
        }
    }

    #[test]
    fn ignores_page_lines_outside_big_files_section() {
        let config = parse("page_lines = 12\n[other]\npage_lines = 34\n").unwrap();

        assert_eq!(config, BigFileConfig::default());
    }

    #[test]
    fn config_path_prefers_xdg_and_falls_back_to_home() {
        assert_eq!(
            config_path(Some("/xdg".as_ref()), Some("/home/cat".as_ref())),
            Some(PathBuf::from("/xdg/catomic/config.toml"))
        );
        assert_eq!(
            config_path(None, Some("/home/cat".as_ref())),
            Some(PathBuf::from("/home/cat/.config/catomic/config.toml"))
        );
        assert_eq!(config_path(None, None), None);
    }

    #[test]
    fn config_path_ignores_empty_or_relative_environment_roots() {
        let home = Some(std::ffi::OsStr::new("/home/cat"));
        let fallback = Some(PathBuf::from("/home/cat/.config/catomic/config.toml"));

        assert_eq!(config_path(Some(std::ffi::OsStr::new("")), home), fallback);
        assert_eq!(
            config_path(Some(std::ffi::OsStr::new("relative-xdg")), home),
            fallback
        );
        assert_eq!(
            config_path(Some(std::ffi::OsStr::new("relative-xdg")), None),
            None
        );
        assert_eq!(
            config_path(None, Some(std::ffi::OsStr::new("relative-home"))),
            None
        );
    }
}
