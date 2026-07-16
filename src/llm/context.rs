//! Purpose: this file must collect bounded, explicit current-buffer LLM context.
//! Owns: selection/file/block drafts, hard size limits, and sensitivity labels.
//! Must not: read files, inspect repos, construct clients, truncate silently, or network.
//! Invariants: every draft has an instruction; over-limit context fails closed.
//! Phase: 6 (LLM, Powerful but Caged).

use std::path::Path;

use super::instruction::{parse_instruction_blocks, InstructionParseError};

pub const MAX_CONTEXT_BYTES: usize = 64 * 1024;
pub const MAX_CONTEXT_LINES: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextScope {
    Selection,
    InstructionBlock,
    CurrentFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    Dotfile,
    SecretLikeLine { line: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestContext {
    pub scope: ContextScope,
    pub text: String,
    pub first_line: usize,
    pub line_count: usize,
    pub byte_count: usize,
    pub sensitivity: Vec<Sensitivity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestDraft {
    pub instruction: String,
    pub context: RequestContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextError {
    EmptyContext,
    MissingInstruction,
    NoInstructionBlockAtCursor,
    InstructionParse(InstructionParseError),
    TooLarge { bytes: usize, lines: usize },
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyContext => write!(formatter, "LLM context is empty"),
            Self::MissingInstruction => write!(formatter, "an explicit instruction is required"),
            Self::NoInstructionBlockAtCursor => {
                write!(formatter, "cursor is not inside a >>> catomic ... <<< block")
            }
            Self::InstructionParse(error) => write!(formatter, "invalid instruction block: {error:?}"),
            Self::TooLarge { bytes, lines } => write!(
                formatter,
                "context is {lines} lines/{bytes} bytes; limit is {MAX_CONTEXT_LINES} lines/{MAX_CONTEXT_BYTES} bytes"
            ),
        }
    }
}

pub fn for_selection(
    text: &str,
    first_line: usize,
    instruction: &str,
    path: Option<&Path>,
) -> Result<RequestDraft, ContextError> {
    build_draft(text, first_line, instruction, ContextScope::Selection, path)
}

pub fn for_current_file(
    text: &str,
    instruction: &str,
    path: Option<&Path>,
) -> Result<RequestDraft, ContextError> {
    build_draft(text, 0, instruction, ContextScope::CurrentFile, path)
}

pub fn for_instruction_block(
    document: &str,
    cursor_line: usize,
    path: Option<&Path>,
) -> Result<RequestDraft, ContextError> {
    let blocks = parse_instruction_blocks(document).map_err(ContextError::InstructionParse)?;
    let block = blocks
        .into_iter()
        .find(|block| (block.start_line..=block.end_line).contains(&cursor_line))
        .ok_or(ContextError::NoInstructionBlockAtCursor)?;
    let text = document
        .lines()
        .skip(block.start_line)
        .take(block.end_line - block.start_line + 1)
        .collect::<Vec<_>>()
        .join("\n");
    build_draft(
        &text,
        block.start_line,
        &block.instruction,
        ContextScope::InstructionBlock,
        path,
    )
}

fn build_draft(
    text: &str,
    first_line: usize,
    instruction: &str,
    scope: ContextScope,
    path: Option<&Path>,
) -> Result<RequestDraft, ContextError> {
    if text.is_empty() {
        return Err(ContextError::EmptyContext);
    }
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err(ContextError::MissingInstruction);
    }
    let byte_count = text.len();
    let line_count = text.split('\n').count();
    if byte_count > MAX_CONTEXT_BYTES || line_count > MAX_CONTEXT_LINES {
        return Err(ContextError::TooLarge {
            bytes: byte_count,
            lines: line_count,
        });
    }
    Ok(RequestDraft {
        instruction: instruction.to_string(),
        context: RequestContext {
            scope,
            text: text.to_string(),
            first_line,
            line_count,
            byte_count,
            sensitivity: sensitivities(text, first_line, path),
        },
    })
}

fn sensitivities(text: &str, first_line: usize, path: Option<&Path>) -> Vec<Sensitivity> {
    let mut found = Vec::new();
    if path.is_some_and(is_dotfile) {
        found.push(Sensitivity::Dotfile);
    }
    for (offset, line) in text.lines().enumerate() {
        if secret_like(line) {
            found.push(Sensitivity::SecretLikeLine {
                line: first_line + offset,
            });
        }
    }
    found
}

fn is_dotfile(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name.starts_with('.') && name != "." && name != ".."
    })
}

fn secret_like(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "password",
        "secret",
        "access_token",
        "private_key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || lower.contains("begin private key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_requires_explicit_instruction_and_reports_exact_extent() {
        assert_eq!(
            for_selection("one\ntwo", 4, " refactor ", Some(Path::new("src/main.rs"))),
            Ok(RequestDraft {
                instruction: "refactor".to_string(),
                context: RequestContext {
                    scope: ContextScope::Selection,
                    text: "one\ntwo".to_string(),
                    first_line: 4,
                    line_count: 2,
                    byte_count: 7,
                    sensitivity: Vec::new(),
                },
            })
        );
        assert_eq!(
            for_selection("one", 0, " ", None),
            Err(ContextError::MissingInstruction)
        );
    }

    #[test]
    fn current_block_supplies_both_instruction_and_bounded_block_context() {
        let document = "code\n>>> catomic\nRefactor this.\nKeep behavior.\n<<<\nafter";
        let draft = for_instruction_block(document, 3, None).unwrap();
        assert_eq!(draft.instruction, "Refactor this.\nKeep behavior.");
        assert_eq!(draft.context.scope, ContextScope::InstructionBlock);
        assert_eq!(draft.context.first_line, 1);
        assert_eq!(
            draft.context.text,
            ">>> catomic\nRefactor this.\nKeep behavior.\n<<<"
        );
    }

    #[test]
    fn hard_limits_fail_closed_instead_of_truncating() {
        let bytes = "x".repeat(MAX_CONTEXT_BYTES + 1);
        assert!(matches!(
            for_current_file(&bytes, "explain", None),
            Err(ContextError::TooLarge { .. })
        ));
        let lines = "x\n".repeat(MAX_CONTEXT_LINES);
        assert!(matches!(
            for_current_file(&lines, "explain", None),
            Err(ContextError::TooLarge { .. })
        ));
    }

    #[test]
    fn labels_dotfiles_and_secret_like_lines_for_explicit_confirmation() {
        let draft = for_current_file(
            "name=cat\nAPI_KEY=do-not-send",
            "explain",
            Some(Path::new("project/.env")),
        )
        .unwrap();
        assert_eq!(
            draft.context.sensitivity,
            [
                Sensitivity::Dotfile,
                Sensitivity::SecretLikeLine { line: 1 }
            ]
        );
    }
}
