//! Purpose: load and persist explicit session-global presentation preferences.
//! Owns: `[view]` defaults, XDG state discovery, precedence, and explicit atomic writes.
//! Must not: inspect buffers, render UI, write during startup, or contact the network.
//! Invariants: persisted toggle state overrides config per key; missing state keeps config/default;
//!   writes use a dedicated owner-only file and occur only after an explicit toggle.
//! Phase: post-v0.1 persistent view preferences.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const DEFAULT_LINE_NUMBERS: bool = false;
const DEFAULT_EXTERNAL_DIFF: bool = true;
const PREFERENCES_FILE: &str = "preferences.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewPreferences {
    line_numbers: bool,
    external_diff: bool,
    path: Option<PathBuf>,
}

impl Default for ViewPreferences {
    fn default() -> Self {
        Self {
            line_numbers: DEFAULT_LINE_NUMBERS,
            external_diff: DEFAULT_EXTERNAL_DIFF,
            path: None,
        }
    }
}

impl ViewPreferences {
    pub(crate) fn line_numbers(&self) -> bool {
        self.line_numbers
    }

    pub(crate) fn set_line_numbers(&mut self, enabled: bool) {
        self.line_numbers = enabled;
    }

    pub(crate) fn external_diff(&self) -> bool {
        self.external_diff
    }

    pub(crate) fn set_external_diff(&mut self, enabled: bool) {
        self.external_diff = enabled;
    }

    pub(crate) fn persist(&self) -> io::Result<()> {
        let path = self.path.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "XDG_STATE_HOME and HOME do not identify an absolute state directory",
            )
        })?;
        persist_to(path, self.line_numbers, self.external_diff)
    }

    #[cfg(test)]
    pub(crate) fn with_path(line_numbers: bool, path: PathBuf) -> Self {
        Self {
            line_numbers,
            external_diff: DEFAULT_EXTERNAL_DIFF,
            path: Some(path),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_values(line_numbers: bool, external_diff: bool, path: PathBuf) -> Self {
        Self {
            line_numbers,
            external_diff,
            path: Some(path),
        }
    }
}

pub(crate) fn current_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME");
    preference_path(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        home.as_deref(),
    )
}

pub(crate) fn load_with_config(
    config: &str,
    preference_path: Option<PathBuf>,
) -> io::Result<ViewPreferences> {
    let configured = parse_config(config)?;
    let persisted =
        read_optional(preference_path.as_deref(), parse_preferences)?.unwrap_or_default();
    Ok(ViewPreferences {
        line_numbers: persisted
            .line_numbers
            .or(configured.line_numbers)
            .unwrap_or(DEFAULT_LINE_NUMBERS),
        external_diff: persisted
            .external_diff
            .or(configured.external_diff)
            .unwrap_or(DEFAULT_EXTERNAL_DIFF),
        path: preference_path,
    })
}

pub(crate) fn validate_config(text: &str) -> io::Result<()> {
    parse_config(text).map(|_| ())
}

#[cfg(test)]
fn load_from_paths(
    config_path: Option<&Path>,
    preference_path: Option<PathBuf>,
) -> io::Result<ViewPreferences> {
    let config = read_optional(config_path, |text| Ok(text.to_string()))?.unwrap_or_default();
    load_with_config(&config, preference_path)
}

fn read_optional<T>(
    path: Option<&Path>,
    parse: fn(&str) -> io::Result<T>,
) -> io::Result<Option<T>> {
    let Some(path) = path else {
        return Ok(None);
    };
    match fs::read_to_string(path) {
        Ok(text) => parse(&text).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_config(text: &str) -> io::Result<ViewSettings> {
    Ok(super::decode::<ViewFile>(text)?.view)
}

fn parse_preferences(text: &str) -> io::Result<ViewSettings> {
    Ok(super::decode::<ViewFile>(text)?.view)
}

#[derive(Default, Deserialize)]
struct ViewFile {
    #[serde(default)]
    view: ViewSettings,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
struct ViewSettings {
    line_numbers: Option<bool>,
    external_diff: Option<bool>,
}

fn persist_to(path: &Path, line_numbers: bool, external_diff: bool) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("preference path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot create preference directory {}: {error}",
                parent.display()
            ),
        )
    })?;
    let contents = format!(
        "# Managed by Catomic after an explicit preference change.\n\
         [view]\nline_numbers = {line_numbers}\nexternal_diff = {external_diff}\n"
    );
    crate::file::io::atomic_write_private_string(path, &contents).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot replace {}: {error}", path.display()),
        )
    })
}

fn preference_path(xdg_state_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let root = xdg_state_home
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| {
            home.map(Path::new)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })?;
    Some(root.join("catomic").join(PREFERENCES_FILE))
}

#[cfg(test)]
#[path = "view_preferences/tests.rs"]
mod tests;
