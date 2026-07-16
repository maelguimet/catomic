//! Purpose: configure bounded line pages for oversized file viewing.
//! Owns: page-line default, minimal TOML-subset parsing, and config path loading.
//! Must not: open editor buffers, scan user files, write config, or know App UI.
//! Invariants: page_lines is nonzero; missing config uses defaults; only the
//!   `[big_files] page_lines = N` setting affects this configuration.
//! Phase: 2-bk configurable paged-file policy.

use std::io;
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_PAGE_LINES: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    let mut config = BigFileConfig::default();
    let mut section = "";
    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            section = name.trim();
            continue;
        }
        if section != "big_files" {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "page_lines" {
            config.page_lines = parse_page_lines(value.trim())?;
        }
    }
    Ok(config)
}

fn parse_page_lines(value: &str) -> io::Result<usize> {
    match value.parse::<usize>() {
        Ok(lines) if lines > 0 => Ok(lines),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "big_files.page_lines must be a positive integer",
        )),
    }
}

pub(crate) fn load_from(path: &Path) -> io::Result<BigFileConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BigFileConfig::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<BigFileConfig> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    match config_path(xdg.as_deref(), home.as_deref()) {
        Some(path) => load_from(&path),
        None => Ok(BigFileConfig::default()),
    }
}

fn config_path(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let root = xdg_config_home
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))?;
    Some(root.join("catomic").join("config.toml"))
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
}
