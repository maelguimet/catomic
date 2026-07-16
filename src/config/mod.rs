//! Purpose: load typed TOML user configuration with safe defaults.
//! Owns: shared TOML decoding and focused configuration submodules.
//! Must not: construct Project/LLM services, perform network work, or mutate files.
//! Invariants: no config file is required; malformed recognized values are errors;
//!   unknown keys are ignored for forward compatibility.
//! Phase: 7 typed configuration foundation.

use std::io;

use serde::de::DeserializeOwned;

pub mod auto_reload;
pub mod big_files;
pub(crate) mod linters;
pub(crate) mod llm;

pub(crate) fn decode<T: DeserializeOwned>(text: &str) -> io::Result<T> {
    toml::from_str(text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
