//! Purpose: this file must describe every bounded context-broker failure.
//! Owns: broker error variants, display text, and low-level error conversion.
//! Must not: perform repository I/O, mutate broker state, write, or network.
//! Invariants: refusals identify their boundary without exposing file contents.
//! Phase: 6 (LLM Context Broker).

use std::fmt;
use std::path::PathBuf;

use crate::project::git::GitError;

#[derive(Debug)]
pub enum BrokerError {
    Git(GitError),
    Discovery(String),
    InvalidPath,
    UnknownFile(PathBuf),
    FileTooLarge { path: PathBuf, bytes: u64 },
    FileChanged(PathBuf),
    SensitiveContent(PathBuf),
    BudgetExceeded { requested: usize, remaining: usize },
    Io(String),
    InvalidUtf8(PathBuf),
    EmptyQuery,
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => write!(formatter, "{error}"),
            Self::Discovery(error) => write!(formatter, "repo discovery failed: {error}"),
            Self::InvalidPath => write!(formatter, "broker paths must be relative and normalized"),
            Self::UnknownFile(path) => write!(
                formatter,
                "file is outside the broker map: {}",
                path.display()
            ),
            Self::FileTooLarge { path, bytes } => write!(
                formatter,
                "broker file is too large ({bytes} bytes): {}",
                path.display()
            ),
            Self::FileChanged(path) => {
                write!(formatter, "relevant file changed: {}", path.display())
            }
            Self::SensitiveContent(path) => write!(
                formatter,
                "broker refused obvious secret-like content in {}",
                path.display()
            ),
            Self::BudgetExceeded {
                requested,
                remaining,
            } => write!(
                formatter,
                "context request needs {requested} bytes; {remaining} remain"
            ),
            Self::Io(error) => write!(formatter, "broker I/O failed: {error}"),
            Self::InvalidUtf8(path) => {
                write!(formatter, "broker file is not UTF-8: {}", path.display())
            }
            Self::EmptyQuery => write!(formatter, "grep query cannot be empty"),
        }
    }
}

impl From<GitError> for BrokerError {
    fn from(error: GitError) -> Self {
        Self::Git(error)
    }
}

pub(super) fn io_error(error: std::io::Error) -> BrokerError {
    BrokerError::Io(error.to_string())
}
