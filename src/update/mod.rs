//! Purpose: coordinate explicit, recoverable Catomic updates outside the editor runtime.
//! Owns: install detection, confirmation, reporting, and stable updater exit codes.
//! Must not: enter terminal raw mode, silently contact a network, or mutate user files.
//! Invariants: checks are read-only; managed/checkout installs retain rollback; user state is immutable.

mod backup;
mod install;
mod managed;
mod process;
mod source;

#[cfg(test)]
mod tests;

use std::fmt;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::cli::{UpdateOptions, UpdateTarget};

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
    let Some(target) = resolve_target(options)? else {
        println!("update cancelled; no network or disk changes made");
        return Ok(());
    };
    match target {
        UpdateTarget::Stable => managed::run(options),
        UpdateTarget::LatestCommit => source::run(options),
    }
}

fn resolve_target(options: UpdateOptions) -> Result<Option<UpdateTarget>, UpdateError> {
    if let Some(target) = options.target {
        return Ok(Some(target));
    }
    if options.assume_yes || options.check {
        return Ok(Some(UpdateTarget::Stable));
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    prompt_target(&mut stdin.lock(), &mut stdout.lock())
}

fn prompt_target(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Option<UpdateTarget>, UpdateError> {
    write!(
        output,
        "Update target:\n  1) latest stable release (default)\n  2) latest official master commit\n  q) cancel\nSelect target [1]: "
    )
    .and_then(|_| output.flush())
    .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, format!("write prompt: {error}")))?;
    let mut response = String::new();
    let read = input
        .read_line(&mut response)
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, format!("read prompt: {error}")))?;
    if read == 0 {
        return Ok(None);
    }
    match response.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "stable" => Ok(Some(UpdateTarget::Stable)),
        "2" | "latest" | "latest-commit" | "master" => Ok(Some(UpdateTarget::LatestCommit)),
        "q" | "quit" | "cancel" => Ok(None),
        response => Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            format!(
                "invalid update target {response:?}; enter 1, 2, or q (no network or disk changes made)"
            ),
        )),
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
