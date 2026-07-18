//! Purpose: decode global editor defaults and extension-specific language settings.
//! Owns: tab width resolution and validated per-language linter commands.
//! Must not: inspect buffers, run commands, construct Project services, or write config.
//! Invariants: tab sizes are 1..=16; extensions are normalized; linters contain `{file}`.
//! Phase: 7 per-language configuration.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Deserialize;

const DEFAULT_TAB_SIZE: usize = 4;
const MAX_TAB_SIZE: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorConfig {
    tab_size: usize,
    languages: BTreeMap<String, LanguageConfig>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LanguageConfig {
    tab_size: Option<usize>,
    linter: Option<String>,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            tab_size: DEFAULT_TAB_SIZE,
            languages: BTreeMap::new(),
        }
    }
}

impl EditorConfig {
    pub(crate) fn tab_size_for_path(&self, path: Option<&Path>) -> usize {
        extension_for_path(path)
            .and_then(|extension| self.languages.get(&extension))
            .and_then(|language| language.tab_size)
            .unwrap_or(self.tab_size)
    }

    pub(crate) fn language_linters(&self) -> impl Iterator<Item = (&str, &str)> {
        self.languages.iter().filter_map(|(extension, language)| {
            language
                .linter
                .as_deref()
                .map(|command| (extension.as_str(), command))
        })
    }
}

pub(crate) fn parse(text: &str) -> io::Result<EditorConfig> {
    let raw = super::decode::<ConfigFile>(text)?;
    validate_tab_size(raw.editor.tab_size, "editor.tab_size")?;
    let mut languages = BTreeMap::new();
    for (raw_extension, language) in raw.languages {
        let extension = normalize_extension(&raw_extension);
        validate_language(&extension, &language)?;
        if languages
            .insert(extension.clone(), language.into())
            .is_some()
        {
            return Err(invalid(format!(
                "language extension {raw_extension:?} duplicates {extension:?}"
            )));
        }
    }
    Ok(EditorConfig {
        tab_size: raw.editor.tab_size,
        languages,
    })
}

pub(crate) fn load_from(path: &Path) -> io::Result<EditorConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(EditorConfig::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<EditorConfig> {
    match super::user_file::optional_path() {
        Some(path) => load_from(&path),
        None => Ok(EditorConfig::default()),
    }
}

#[derive(Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    editor: RawEditorConfig,
    #[serde(default)]
    languages: BTreeMap<String, RawLanguageConfig>,
}

#[derive(Deserialize)]
#[serde(default)]
struct RawEditorConfig {
    tab_size: usize,
}

impl Default for RawEditorConfig {
    fn default() -> Self {
        Self {
            tab_size: DEFAULT_TAB_SIZE,
        }
    }
}

#[derive(Default, Deserialize)]
struct RawLanguageConfig {
    tab_size: Option<usize>,
    linter: Option<String>,
}

impl From<RawLanguageConfig> for LanguageConfig {
    fn from(raw: RawLanguageConfig) -> Self {
        Self {
            tab_size: raw.tab_size,
            linter: raw.linter,
        }
    }
}

fn validate_language(extension: &str, language: &RawLanguageConfig) -> io::Result<()> {
    if extension.is_empty() || extension.chars().any(char::is_whitespace) {
        return Err(invalid("language extension must not be empty"));
    }
    if let Some(tab_size) = language.tab_size {
        validate_tab_size(tab_size, &format!("languages.{extension}.tab_size"))?;
    }
    if language
        .linter
        .as_deref()
        .is_some_and(|command| !command.contains("{file}"))
    {
        return Err(invalid(format!(
            "languages.{extension}.linter must contain {{file}}"
        )));
    }
    Ok(())
}

fn validate_tab_size(tab_size: usize, name: &str) -> io::Result<()> {
    if !(1..=MAX_TAB_SIZE).contains(&tab_size) {
        return Err(invalid(format!("{name} must be between 1 and 16")));
    }
    Ok(())
}

fn extension_for_path(path: Option<&Path>) -> Option<String> {
    path.and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(normalize_extension)
}

fn normalize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_configuration_uses_four_space_tabs() {
        let config = EditorConfig::default();

        assert_eq!(config.tab_size_for_path(None), 4);
        assert_eq!(config.tab_size_for_path(Some(Path::new("main.rs"))), 4);
    }

    #[test]
    fn language_tab_size_overrides_global_default() {
        let config = parse(
            "[editor]\ntab_size = 3\n[languages.rs]\ntab_size = 4\n[languages.py]\ntab_size = 2\n",
        )
        .unwrap();

        assert_eq!(config.tab_size_for_path(Some(Path::new("main.RS"))), 4);
        assert_eq!(config.tab_size_for_path(Some(Path::new("tool.py"))), 2);
        assert_eq!(config.tab_size_for_path(Some(Path::new("notes.txt"))), 3);
    }

    #[test]
    fn language_linter_is_validated_and_exposed() {
        let config = parse("[languages.rs]\nlinter = \"cargo check {file}\"\n").unwrap();
        let linters: Vec<_> = config.language_linters().collect();

        assert_eq!(linters, vec![("rs", "cargo check {file}")]);
        assert!(parse("[languages.rs]\nlinter = \"cargo check\"\n").is_err());
    }

    #[test]
    fn rejects_unbounded_tabs_and_duplicate_normalized_extensions() {
        for text in [
            "[editor]\ntab_size = 0\n",
            "[languages.rs]\ntab_size = 17\n",
            "[languages.rs]\ntab_size = 2\n[languages.\".rs\"]\ntab_size = 4\n",
        ] {
            assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
