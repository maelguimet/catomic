//! Purpose: configure automatic reload of clean buffers after external edits.
//! Owns: the default-on flag, minimal TOML-subset parsing, and config loading.
//! Must not: construct watchers, open editor buffers, write config, or know UI.
//! Invariants: missing config enables auto reload; only
//!   `[files] auto_reload = true|false` affects this setting.
//! Phase: 2-bx automatic external reload policy.

use std::io;
use std::path::Path;

pub(crate) const DEFAULT_AUTO_RELOAD: bool = true;

pub(crate) fn parse(text: &str) -> io::Result<bool> {
    let mut auto_reload = DEFAULT_AUTO_RELOAD;
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
        if section != "files" {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "auto_reload" {
            auto_reload = parse_bool(value.trim())?;
        }
    }
    Ok(auto_reload)
}

fn parse_bool(value: &str) -> io::Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "files.auto_reload must be true or false",
        )),
    }
}

pub(crate) fn load_from(path: &Path) -> io::Result<bool> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(DEFAULT_AUTO_RELOAD),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<bool> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    match super::big_files::config_path(xdg.as_deref(), home.as_deref()) {
        Some(path) => load_from(&path),
        None => Ok(DEFAULT_AUTO_RELOAD),
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
