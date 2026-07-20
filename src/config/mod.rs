//! Purpose: load typed TOML user configuration with safe defaults.
//! Owns: shared TOML decoding and focused configuration submodules.
//! Must not: construct linter/LLM services, perform network work, or mutate files.
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
pub(crate) mod llm;
pub(crate) mod mobile;
pub(crate) mod theme;
pub(crate) mod user_file;
mod validation;
pub(crate) mod view_preferences;

pub(crate) use validation::validate_unknown_keys;

pub(crate) fn decode<T: DeserializeOwned>(text: &str) -> io::Result<T> {
    toml::from_str(text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn validate_all() -> io::Result<()> {
    let text = user_file::read_optional()?.unwrap_or_default();
    validate_text(&text)
}

pub(crate) fn validate_text(text: &str) -> io::Result<()> {
    validate_unknown_keys(text)?;
    auto_reload::parse(text)?;
    big_files::parse(text)?;
    cat::parse(text)?;
    commands::parse(text)?;
    editor::parse(text)?;
    keybindings::parse(text)?;
    linters::parse(text)?;
    llm::parse(text)?;
    mobile::parse(text)?;
    theme::parse(text)?;
    view_preferences::validate_config(text)?;
    Ok(())
}
