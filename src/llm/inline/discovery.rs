//! Purpose: implement deterministic inline instruction and context-block discovery.
//! Owns: line parsing, exact captured ranges, metadata stripping, and request formatting.
//! Must not: perform I/O, call models, mutate text, or silently recover malformed delimiters.
//! Invariants: all diagnostics are one-based; request context excludes every control delimiter.

use std::path::Path;

use crate::buffer::Cursor;
use crate::config::llm::InlineSettings;
use crate::llm::context;
use crate::llm::instruction::{parse_instruction_blocks, InstructionParseError};

use super::{CapturedRange, ContextTarget, InlineDraft, InlineError, InstructionMetadata};

#[derive(Clone)]
struct Line<'a> {
    index: usize,
    byte_start: usize,
    content_end: usize,
    full_end: usize,
    text: &'a str,
}

#[derive(Clone)]
struct Candidate {
    instruction: InstructionMetadata,
    start_line: usize,
    end_line: usize,
}

pub(super) fn discover(
    document: &str,
    cursor_line: usize,
    selection: Option<(Cursor, Cursor)>,
    path: Option<&Path>,
    settings: &InlineSettings,
) -> Result<InlineDraft, InlineError> {
    let lines = source_lines(document);
    let anchor = selection.map_or(cursor_line, |(start, end)| ordered(start, end).0.row);
    let instruction = choose_instruction(document, &lines, anchor, settings)?;
    validate_instruction_size(&instruction)?;
    if let Some(selection) = selection {
        let (start, end) = ordered(selection.0, selection.1);
        if ranges_overlap(
            start,
            end,
            instruction.metadata.start,
            instruction.metadata.end,
        ) {
            return Err(InlineError::SelectionContainsInstruction {
                line: instruction.display_line,
            });
        }
        let mut draft = super::draft::selection(
            slice_range(document, &lines, start, end).to_string(),
            (start, end),
            instruction,
            Vec::new(),
            path,
            (lines.len(), document.len()),
        )?;
        add_instruction_sensitivity(&mut draft);
        return Ok(draft);
    }

    let (block_targets, delimiter_guards) = parse_context_blocks(document, &lines, settings)?;
    if block_targets.iter().any(|target| {
        ranges_overlap(
            target.range.start,
            target.range.end,
            instruction.metadata.start,
            instruction.metadata.end,
        )
    }) {
        return Err(InlineError::SelectionContainsInstruction {
            line: instruction.display_line,
        });
    }

    let mut draft = if !block_targets.is_empty() {
        super::draft::blocks(
            block_targets,
            delimiter_guards,
            instruction,
            path,
            settings,
            lines.len(),
            document.len(),
        )?
    } else {
        super::draft::full_file(
            document,
            instruction.clone(),
            path,
            byte_at(&lines, instruction.metadata.start),
            byte_at(&lines, instruction.metadata.end),
            lines.len(),
        )?
    };
    add_instruction_sensitivity(&mut draft);
    Ok(draft)
}

fn validate_instruction_size(instruction: &InstructionMetadata) -> Result<(), InlineError> {
    let bytes = instruction.text.len();
    let lines = instruction.text.split('\n').count();
    if bytes > context::MAX_CONTEXT_BYTES || lines > context::MAX_CONTEXT_LINES {
        return Err(InlineError::Context(context::ContextError::TooLarge {
            bytes,
            lines,
        }));
    }
    Ok(())
}

fn add_instruction_sensitivity(draft: &mut InlineDraft) {
    if context::secret_like(&draft.instruction.text) {
        let warning = context::Sensitivity::SecretLikeLine {
            line: draft.instruction.display_line - 1,
        };
        if !draft.sensitivity.contains(&warning) {
            draft.sensitivity.push(warning);
        }
    }
}

fn choose_instruction(
    document: &str,
    lines: &[Line<'_>],
    anchor: usize,
    settings: &InlineSettings,
) -> Result<InstructionMetadata, InlineError> {
    let mut candidates = inline_candidates(document, lines, settings)?;
    candidates.extend(legacy_candidates(document, lines)?);
    for (index, left) in candidates.iter().enumerate() {
        for right in &candidates[index + 1..] {
            if ranges_overlap(
                left.instruction.metadata.start,
                left.instruction.metadata.end,
                right.instruction.metadata.start,
                right.instruction.metadata.end,
            ) {
                return Err(InlineError::AmbiguousInstruction {
                    lines: vec![
                        left.instruction.display_line,
                        right.instruction.display_line,
                    ],
                });
            }
        }
    }
    let containing: Vec<_> = candidates
        .iter()
        .filter(|candidate| (candidate.start_line..=candidate.end_line).contains(&anchor))
        .collect();
    if containing.len() == 1 {
        return Ok(containing[0].instruction.clone());
    }
    if containing.len() > 1 {
        return Err(InlineError::AmbiguousInstruction {
            lines: containing
                .iter()
                .map(|candidate| candidate.instruction.display_line)
                .collect(),
        });
    }
    let nearest_line = candidates
        .iter()
        .filter(|candidate| candidate.end_line <= anchor)
        .map(|candidate| candidate.end_line)
        .max()
        .ok_or(InlineError::MissingInstruction)?;
    let nearest: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.end_line == nearest_line)
        .collect();
    if nearest.len() != 1 {
        return Err(InlineError::AmbiguousInstruction {
            lines: nearest
                .iter()
                .map(|candidate| candidate.instruction.display_line)
                .collect(),
        });
    }
    Ok(nearest[0].instruction.clone())
}

fn inline_candidates(
    document: &str,
    lines: &[Line<'_>],
    settings: &InlineSettings,
) -> Result<Vec<Candidate>, InlineError> {
    let mut candidates = Vec::new();
    for line in lines {
        let trimmed = line.text.trim();
        let Some(rest) = trimmed.strip_prefix(&settings.instruction_prefix) else {
            continue;
        };
        if !rest.is_empty() && !rest.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        let mut instruction = rest.trim();
        if !settings.instruction_suffix.is_empty() {
            instruction = instruction
                .strip_suffix(&settings.instruction_suffix)
                .ok_or(InlineError::MissingInstructionSuffix {
                    line: line.index + 1,
                })?
                .trim_end();
        }
        if instruction.is_empty() {
            return Err(InlineError::EmptyInstruction {
                line: line.index + 1,
            });
        }
        let metadata = captured_lines(document, lines, line.index, line.index);
        let cleanup = cleanup_lines(document, lines, line.index, line.index);
        candidates.push(Candidate {
            instruction: InstructionMetadata {
                text: instruction.to_string(),
                display_line: line.index + 1,
                metadata,
                cleanup,
                legacy_block: false,
            },
            start_line: line.index,
            end_line: line.index,
        });
    }
    Ok(candidates)
}

fn legacy_candidates(document: &str, lines: &[Line<'_>]) -> Result<Vec<Candidate>, InlineError> {
    let blocks = parse_instruction_blocks(document).map_err(convert_legacy_error)?;
    Ok(blocks
        .into_iter()
        .map(|block| Candidate {
            instruction: InstructionMetadata {
                text: block.instruction.trim().to_string(),
                display_line: block.start_line + 1,
                metadata: captured_lines(document, lines, block.start_line, block.end_line),
                cleanup: cleanup_lines(document, lines, block.start_line, block.end_line),
                legacy_block: true,
            },
            start_line: block.start_line,
            end_line: block.end_line,
        })
        .collect())
}

fn convert_legacy_error(error: InstructionParseError) -> InlineError {
    match error {
        InstructionParseError::NestedStart { line } => InlineError::MalformedLegacyInstruction {
            line: line + 1,
            message: "nested >>> catomic marker",
        },
        InstructionParseError::UnexpectedEnd { line } => InlineError::MalformedLegacyInstruction {
            line: line + 1,
            message: "unexpected <<< marker",
        },
        InstructionParseError::UnclosedBlock { start_line } => {
            InlineError::MalformedLegacyInstruction {
                line: start_line + 1,
                message: "unclosed >>> catomic block",
            }
        }
    }
}

fn parse_context_blocks(
    document: &str,
    lines: &[Line<'_>],
    settings: &InlineSettings,
) -> Result<(Vec<ContextTarget>, Vec<CapturedRange>), InlineError> {
    let mut open = None;
    let mut targets = Vec::new();
    let mut guards = Vec::new();
    for line in lines {
        let trimmed = line.text.trim();
        if trimmed == settings.context_open {
            if let Some(open_line) = open {
                return Err(InlineError::NestedContextOpen {
                    line: line.index + 1,
                    open_line: open_line + 1,
                });
            }
            open = Some(line.index);
        } else if trimmed == settings.context_close {
            let Some(open_line) = open.take() else {
                return Err(InlineError::UnexpectedContextClose {
                    line: line.index + 1,
                });
            };
            let start = Cursor {
                row: open_line + 1,
                col: 0,
            };
            let end = Cursor {
                row: line.index,
                col: 0,
            };
            if start == end {
                return Err(InlineError::EmptyContextBlock {
                    line: open_line + 1,
                });
            }
            let original = slice_range(document, lines, start, end).to_string();
            if original.is_empty() {
                return Err(InlineError::EmptyContextBlock {
                    line: open_line + 1,
                });
            }
            let id = targets.len() + 1;
            targets.push(ContextTarget {
                id,
                range: CapturedRange {
                    start,
                    end,
                    original,
                    first_line: open_line + 1,
                    last_line: line.index.saturating_sub(1),
                },
            });
            guards.push(captured_lines(document, lines, open_line, open_line));
            guards.push(captured_lines(document, lines, line.index, line.index));
        }
    }
    if let Some(open_line) = open {
        return Err(InlineError::UnclosedContext {
            line: open_line + 1,
        });
    }
    Ok((targets, guards))
}

fn source_lines(document: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut byte_start = 0;
    for (index, newline) in document.match_indices('\n') {
        lines.push(Line {
            index: lines.len(),
            byte_start,
            content_end: index,
            full_end: newline.len() + index,
            text: &document[byte_start..index],
        });
        byte_start = index + newline.len();
    }
    lines.push(Line {
        index: lines.len(),
        byte_start,
        content_end: document.len(),
        full_end: document.len(),
        text: &document[byte_start..],
    });
    lines
}

fn captured_lines(
    document: &str,
    lines: &[Line<'_>],
    start_line: usize,
    end_line: usize,
) -> CapturedRange {
    let start = Cursor {
        row: start_line,
        col: 0,
    };
    let end = Cursor {
        row: end_line,
        col: lines[end_line].text.chars().count(),
    };
    CapturedRange {
        start,
        end,
        original: slice_range(document, lines, start, end).to_string(),
        first_line: start_line,
        last_line: end_line,
    }
}

fn cleanup_lines(
    document: &str,
    lines: &[Line<'_>],
    start_line: usize,
    end_line: usize,
) -> CapturedRange {
    let (start, end) = if lines[end_line].full_end > lines[end_line].content_end {
        (
            Cursor {
                row: start_line,
                col: 0,
            },
            Cursor {
                row: end_line + 1,
                col: 0,
            },
        )
    } else if start_line > 0 {
        (
            Cursor {
                row: start_line - 1,
                col: lines[start_line - 1].text.chars().count(),
            },
            Cursor {
                row: end_line,
                col: lines[end_line].text.chars().count(),
            },
        )
    } else {
        (
            Cursor::default(),
            Cursor {
                row: end_line,
                col: lines[end_line].text.chars().count(),
            },
        )
    };
    CapturedRange {
        start,
        end,
        original: slice_range(document, lines, start, end).to_string(),
        first_line: start_line,
        last_line: end_line,
    }
}

fn byte_at(lines: &[Line<'_>], cursor: Cursor) -> usize {
    let line = &lines[cursor.row.min(lines.len().saturating_sub(1))];
    line.byte_start
        + line
            .text
            .char_indices()
            .nth(cursor.col)
            .map_or(line.text.len(), |(byte, _)| byte)
}

fn slice_range<'a>(document: &'a str, lines: &[Line<'_>], start: Cursor, end: Cursor) -> &'a str {
    &document[byte_at(lines, start)..byte_at(lines, end)]
}

fn ordered(left: Cursor, right: Cursor) -> (Cursor, Cursor) {
    if (left.row, left.col) <= (right.row, right.col) {
        (left, right)
    } else {
        (right, left)
    }
}

fn ranges_overlap(
    left_start: Cursor,
    left_end: Cursor,
    right_start: Cursor,
    right_end: Cursor,
) -> bool {
    (left_start.row, left_start.col) < (right_end.row, right_end.col)
        && (right_start.row, right_start.col) < (left_end.row, left_end.col)
}
