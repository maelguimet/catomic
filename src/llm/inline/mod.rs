//! Purpose: discover and validate bounded inline-clanker instructions and edit scopes.
//! Owns: request-local instruction metadata, catblock targets, scope precedence, and envelopes.
//! Must not: read files, construct clients, mutate buffers, render UI, or relax hard limits.
//! Invariants: control markers are never targets; every target is exact and bounded.
//! Phase: issue #65 one-key inline clanker workflow.

use std::path::Path;

use crate::buffer::Cursor;
use crate::config::llm::{InlineBlockMode, InlineSettings};
use crate::llm::context::{ContextError, Sensitivity};

mod discovery;
mod draft;
mod response;

pub use response::parse_combined_replacements;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineScope {
    Selection,
    Blocks,
    FullFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRange {
    pub start: Cursor,
    pub end: Cursor,
    pub original: String,
    pub first_line: usize,
    pub last_line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionMetadata {
    pub text: String,
    pub display_line: usize,
    pub metadata: CapturedRange,
    pub cleanup: CapturedRange,
    pub legacy_block: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextTarget {
    pub id: usize,
    pub range: CapturedRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestUnit {
    pub target_ids: Vec<usize>,
    pub text: String,
    pub line_count: usize,
    pub byte_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineDraft {
    pub instruction: InstructionMetadata,
    pub scope: InlineScope,
    pub targets: Vec<ContextTarget>,
    pub delimiter_guards: Vec<CapturedRange>,
    pub requests: Vec<RequestUnit>,
    pub sensitivity: Vec<Sensitivity>,
    pub full_file_sentinel: Option<String>,
    pub full_file_lines: usize,
    pub full_file_bytes: usize,
}

impl InlineDraft {
    pub fn target(&self, id: usize) -> Option<&ContextTarget> {
        self.targets.iter().find(|target| target.id == id)
    }

    pub fn block_mode_label(&self, settings: &InlineSettings) -> &'static str {
        match (self.scope, settings.block_mode) {
            (InlineScope::Blocks, InlineBlockMode::Queued) => "queued",
            _ => "combined",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineError {
    MissingInstruction,
    EmptyInstruction { line: usize },
    MissingInstructionSuffix { line: usize },
    AmbiguousInstruction { lines: Vec<usize> },
    MalformedLegacyInstruction { line: usize, message: &'static str },
    UnexpectedContextClose { line: usize },
    NestedContextOpen { line: usize, open_line: usize },
    UnclosedContext { line: usize },
    EmptyContextBlock { line: usize },
    SelectionContainsInstruction { line: usize },
    QueueLimit { blocks: usize, limit: usize },
    Context(ContextError),
}

impl std::fmt::Display for InlineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingInstruction => write!(
                formatter,
                "no inline instruction was found at or before the cursor/selection"
            ),
            Self::EmptyInstruction { line } => {
                write!(formatter, "instruction marker on line {line} has no instruction")
            }
            Self::MissingInstructionSuffix { line } => write!(
                formatter,
                "instruction marker on line {line} is missing its configured suffix"
            ),
            Self::AmbiguousInstruction { lines } => write!(
                formatter,
                "ambiguous instruction markers on lines {}",
                lines
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::MalformedLegacyInstruction { line, message } => {
                write!(formatter, "legacy instruction block error on line {line}: {message}")
            }
            Self::UnexpectedContextClose { line } => {
                write!(formatter, "unexpected context closing delimiter on line {line}")
            }
            Self::NestedContextOpen { line, open_line } => write!(
                formatter,
                "nested context delimiter on line {line}; block opened on line {open_line}"
            ),
            Self::UnclosedContext { line } => {
                write!(formatter, "context block opened on line {line} is not closed")
            }
            Self::EmptyContextBlock { line } => {
                write!(formatter, "context block opened on line {line} is empty")
            }
            Self::SelectionContainsInstruction { line } => write!(
                formatter,
                "selection includes the instruction metadata on line {line}; select only editable text"
            ),
            Self::QueueLimit { blocks, limit } => write!(
                formatter,
                "queued mode found {blocks} blocks; configured queue limit is {limit}"
            ),
            Self::Context(error) => error.fmt(formatter),
        }
    }
}

impl From<ContextError> for InlineError {
    fn from(error: ContextError) -> Self {
        Self::Context(error)
    }
}

pub fn discover(
    document: &str,
    cursor_line: usize,
    selection: Option<(Cursor, Cursor)>,
    path: Option<&Path>,
    settings: &InlineSettings,
) -> Result<InlineDraft, InlineError> {
    discovery::discover(document, cursor_line, selection, path, settings)
}

#[cfg(test)]
mod tests;
