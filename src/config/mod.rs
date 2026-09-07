//! Purpose: load typed TOML user configuration with safe defaults.
//! Owns: shared TOML decoding and focused configuration submodules.
//! Must not: construct linter services, perform network work, or mutate files.
//! Invariants: no config file is required; malformed recognized values and unknown keys are errors.

use std::io;

use serde::de::DeserializeOwned;

pub(crate) mod actions;
pub mod auto_reload;
pub mod big_files;
pub(crate) mod cat;
pub(crate) mod commands;
pub(crate) mod editor;
pub(crate) mod keybindings;
pub(crate) mod linters;
pub(crate) mod mobile;
pub(crate) mod theme;
pub(crate) mod user_file;
mod validation;
pub(crate) mod view_preferences;

pub(crate) fn decode<T: DeserializeOwned>(text: &str) -> io::Result<T> {
    toml::from_str(text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn validate_all() -> io::Result<()> {
    let text = user_file::read_optional()?.unwrap_or_default();
    validate_text(&text)
}

pub(crate) fn validate_text(text: &str) -> io::Result<()> {
    let document = Document::parse(text)?;
    document.validate_unknown_keys()?;
    auto_reload::from_document(&document)?;
    big_files::from_document(&document)?;
    cat::from_document(&document)?;
    commands::from_document(&document)?;
    editor::from_document(&document)?;
    keybindings::from_document(&document)?;
    linters::from_document(&document)?;
    mobile::from_document(&document)?;
    theme::from_document(&document)?;
    view_preferences::validate_config(&document)?;
    Ok(())
}

/// One source parse, shared by independent section validators. Keep the original
/// spans so typed errors still show the same source line and column.
pub(crate) struct Document<'a> {
    text: &'a str,
    root: toml::Spanned<toml::de::DeTable<'a>>,
}

impl<'a> Document<'a> {
    pub(crate) fn parse(text: &'a str) -> io::Result<Self> {
        let root = toml::de::DeTable::parse(text).map_err(invalid_toml)?;
        Ok(Self { text, root })
    }

    pub(crate) fn validate_unknown_keys(&self) -> io::Result<()> {
        let root = self.deserialize::<toml::Table>(self.root.clone())?;
        validation::validate_table(&root)
    }

    pub(super) fn decode<T: DeserializeOwned>(&self, sections: &[&str]) -> io::Result<T> {
        // TOML's deserializer consumes its tree. Copy only the sections this
        // reader owns, rather than cloning unrelated (potentially large) data.
        let root = self
            .root
            .get_ref()
            .iter()
            .filter(|(key, _)| sections.contains(&key.get_ref().as_ref()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.deserialize(toml::Spanned::new(self.root.span(), root))
    }

    fn deserialize<T: DeserializeOwned>(
        &self,
        root: toml::Spanned<toml::de::DeTable<'a>>,
    ) -> io::Result<T> {
        T::deserialize(toml::de::Deserializer::from(root)).map_err(|mut error| {
            error.set_input(Some(self.text));
            invalid_toml(error)
        })
    }
}

fn invalid_toml(error: toml::de::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
