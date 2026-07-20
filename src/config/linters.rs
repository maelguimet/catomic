//! Purpose: parse on-demand extension-to-command mappings from `[linters]`.
//! Owns: linter command validation, extension normalization, and lazy config-file loading.
//! Must not: run commands, construct services, load during ordinary startup, or write config.
//! Invariants: every accepted command contains `{file}`; missing config yields an empty mapping.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinterConfig {
    commands: BTreeMap<String, String>,
}

impl LinterConfig {
    pub(crate) fn command_for_extension(&self, extension: &str) -> Option<&str> {
        self.commands
            .get(&normalize_extension(extension))
            .map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

pub(crate) fn parse(text: &str) -> io::Result<LinterConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        linters: BTreeMap<String, String>,
    }

    let mut config = LinterConfig::default();
    for (raw_extension, command) in super::decode::<ConfigFile>(text)?.linters {
        let extension = normalize_extension(&raw_extension);
        if extension.is_empty() || extension.chars().any(char::is_whitespace) {
            return Err(invalid("linter extension must not be empty"));
        }
        if !command.contains("{file}") {
            return Err(invalid("linter command must contain {file}"));
        }
        config.commands.insert(extension, command);
    }
    for (extension, command) in super::editor::parse(text)?.language_linters() {
        config
            .commands
            .insert(extension.to_string(), command.to_string());
    }
    Ok(config)
}

pub(crate) fn load_from(path: &Path) -> io::Result<LinterConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LinterConfig::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<LinterConfig> {
    match super::user_file::optional_path() {
        Some(path) => load_from(&path),
        None => Ok(LinterConfig::default()),
    }
}

fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_extension_mappings_and_normalizes_dots() {
        let config = parse(
            "[linters]\n\".rs\" = \"rustc --error-format short {file}\"\npy = 'ruff check {file}'\n",
        )
        .unwrap();

        assert_eq!(
            config.command_for_extension("rs"),
            Some("rustc --error-format short {file}")
        );
        assert_eq!(
            config.command_for_extension(".py"),
            Some("ruff check {file}")
        );
    }

    #[test]
    fn ignores_other_sections_and_rejects_missing_placeholder() {
        assert!(parse("[other]\nrs = \"tool {file}\"\n").unwrap().is_empty());

        let error = parse("[linters]\nrs = \"cargo check\"\n").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("{file}"));
    }

    #[test]
    fn rejects_empty_extensions_and_non_string_commands() {
        for text in [
            "[linters]\n\".\" = \"tool {file}\"\n",
            "[linters]\nrs = 42\n",
        ] {
            assert_eq!(
                parse(text).unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn per_language_linter_overrides_legacy_mapping() {
        let config = parse(
            "[linters]\nrs = \"legacy {file}\"\n[languages.rs]\nlinter = \"language {file}\"\n",
        )
        .unwrap();

        assert_eq!(config.command_for_extension("rs"), Some("language {file}"));
    }
}
