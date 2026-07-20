//! Purpose: coordinate explicit, recoverable Catomic updates outside the editor runtime.
//! Owns: install detection, confirmation, reporting, and stable updater exit codes.
//! Must not: enter terminal raw mode, silently contact a network, or mutate user files.
//! Invariants: checks are read-only; managed/checkout installs retain rollback; user state is immutable.
//! Phase: safe self-update workflow.

mod backup;
mod install;
mod managed;
mod process;
mod source;

#[cfg(test)]
mod tests;

use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::cli::UpdateOptions;

pub(crate) const EXIT_UNSUPPORTED: i32 = 3;
pub(crate) const EXIT_NETWORK: i32 = 4;
pub(crate) const EXIT_SOURCE_STATE: i32 = 5;
pub(crate) const EXIT_BACKUP: i32 = 6;
pub(crate) const EXIT_CONFIG: i32 = 7;
pub(crate) const EXIT_BUILD: i32 = 8;
pub(crate) const EXIT_INSTALL: i32 = 9;

#[derive(Debug)]
pub(crate) struct UpdateError {
    code: i32,
    message: String,
}

impl UpdateError {
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn exit_code(&self) -> i32 {
        self.code
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(crate) fn run(options: UpdateOptions) -> Result<(), UpdateError> {
    if managed::is_managed_build() {
        managed::run(options)
    } else {
        source::run(options)
    }
}

fn confirm(options: UpdateOptions, prompt: &str) -> Result<bool, UpdateError> {
    if options.assume_yes {
        println!("confirmation: accepted by --yes");
        return Ok(true);
    }
    print!("{prompt} [y/N] ");
    io::stdout()
        .flush()
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, format!("write prompt: {error}")))?;
    let mut response = String::new();
    io::stdin()
        .read_line(&mut response)
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, format!("read prompt: {error}")))?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn maybe_backup(options: UpdateOptions) -> Result<Option<PathBuf>, UpdateError> {
    if !options.backup {
        return Ok(None);
    }
    let path = backup::create(env!("CARGO_PKG_VERSION"))
        .map_err(|error| UpdateError::new(EXIT_BACKUP, error))?;
    println!("backup: {}", path.display());
    Ok(Some(path))
}

fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}
