//! Purpose: build bounded inline-clanker request units from already parsed source ranges.
//! Owns: selection/block/full-file context formatting, hard-limit checks, and queue shaping.
//! Must not: parse markers, choose instructions, perform I/O, mutate text, or call a model.
//! Invariants: request text contains only the selected scope and every target has a stable ID.

use std::path::Path;

use crate::buffer::Cursor;
use crate::config::llm::{InlineBlockMode, InlineSettings};
use crate::llm::context;

use super::{
    CapturedRange, ContextTarget, InlineDraft, InlineError, InlineScope, InstructionMetadata,
    RequestUnit,
};

pub(super) fn selection(
    original: String,
    range: (Cursor, Cursor),
    instruction: InstructionMetadata,
    delimiter_guards: Vec<CapturedRange>,
    path: Option<&Path>,
    full_file_size: (usize, usize),
) -> Result<InlineDraft, InlineError> {
    let (start, end) = range;
    let (full_file_lines, full_file_bytes) = full_file_size;
    let first_line = start.row;
    let last_line = last_covered_line(start, end);
    let context = context::for_selection(&original, first_line, &instruction.text, path)?;
    Ok(InlineDraft {
        instruction,
        scope: InlineScope::Selection,
        targets: vec![ContextTarget {
            id: 1,
            range: CapturedRange {
                start,
                end,
                original,
                first_line,
                last_line,
            },
        }],
        delimiter_guards,
        requests: vec![RequestUnit {
            target_ids: vec![1],
            text: context.context.text,
            line_count: context.context.line_count,
            byte_count: context.context.byte_count,
        }],
        sensitivity: context.context.sensitivity,
        full_file_sentinel: None,
        full_file_lines,
        full_file_bytes,
    })
}

pub(super) fn blocks(
    targets: Vec<ContextTarget>,
    delimiter_guards: Vec<CapturedRange>,
    instruction: InstructionMetadata,
    path: Option<&Path>,
    settings: &InlineSettings,
    full_file_lines: usize,
    full_file_bytes: usize,
) -> Result<InlineDraft, InlineError> {
    if settings.block_mode == InlineBlockMode::Queued && targets.len() > settings.queue_limit {
        return Err(InlineError::QueueLimit {
            blocks: targets.len(),
            limit: settings.queue_limit,
        });
    }
    let mut sensitivity = Vec::new();
    for target in &targets {
        let context = context::for_selection(
            &target.range.original,
            target.range.first_line,
            &instruction.text,
            path,
        )?;
        for warning in context.context.sensitivity {
            if !sensitivity.contains(&warning) {
                sensitivity.push(warning);
            }
        }
    }
    let requests = if settings.block_mode == InlineBlockMode::Queued && targets.len() > 1 {
        targets
            .iter()
            .map(|target| request_unit(std::slice::from_ref(target), &instruction.text, path))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![request_unit(&targets, &instruction.text, path)?]
    };
    Ok(InlineDraft {
        instruction,
        scope: InlineScope::Blocks,
        targets,
        delimiter_guards,
        requests,
        sensitivity,
        full_file_sentinel: None,
        full_file_lines,
        full_file_bytes,
    })
}

pub(super) fn full_file(
    document: &str,
    instruction: InstructionMetadata,
    path: Option<&Path>,
    metadata_start: usize,
    metadata_end: usize,
    full_file_lines: usize,
) -> Result<InlineDraft, InlineError> {
    if document.len() > context::MAX_CONTEXT_BYTES || full_file_lines > context::MAX_CONTEXT_LINES {
        return Err(InlineError::Context(context::ContextError::TooLarge {
            bytes: document.len(),
            lines: full_file_lines,
        }));
    }
    let sentinel = unique_sentinel(document);
    let mut text = String::with_capacity(document.len().saturating_add(sentinel.len()));
    text.push_str(&document[..metadata_start]);
    text.push_str(&sentinel);
    text.push_str(&document[metadata_end..]);
    let checked = context::for_current_file(&text, &instruction.text, path)?;
    Ok(InlineDraft {
        instruction,
        scope: InlineScope::FullFile,
        targets: Vec::new(),
        delimiter_guards: Vec::new(),
        requests: vec![RequestUnit {
            target_ids: Vec::new(),
            text: checked.context.text,
            line_count: checked.context.line_count,
            byte_count: checked.context.byte_count,
        }],
        sensitivity: checked.context.sensitivity,
        full_file_sentinel: Some(sentinel),
        full_file_lines,
        full_file_bytes: document.len(),
    })
}

fn request_unit(
    targets: &[ContextTarget],
    instruction: &str,
    path: Option<&Path>,
) -> Result<RequestUnit, InlineError> {
    let text = if targets.len() == 1 {
        targets[0].range.original.clone()
    } else {
        numbered_context(targets)
    };
    let checked = context::for_current_file(&text, instruction, path)?;
    Ok(RequestUnit {
        target_ids: targets.iter().map(|target| target.id).collect(),
        text,
        line_count: checked.context.line_count,
        byte_count: checked.context.byte_count,
    })
}

fn numbered_context(targets: &[ContextTarget]) -> String {
    let mut formatted = String::new();
    for (index, target) in targets.iter().enumerate() {
        if index > 0 {
            formatted.push('\n');
        }
        formatted.push_str(&format!(
            "[Context block {} of {}; source lines {}-{}]\n",
            target.id,
            targets.len(),
            target.range.first_line + 1,
            target.range.last_line + 1
        ));
        formatted.push_str(&target.range.original);
        if !target.range.original.ends_with('\n') {
            formatted.push('\n');
        }
        formatted.push_str(&format!("[End context block {}]", target.id));
        if index + 1 < targets.len() {
            formatted.push('\n');
        }
    }
    formatted
}

fn unique_sentinel(document: &str) -> String {
    for index in 1..=1_000 {
        let sentinel = format!("[[CATOMIC-INSTRUCTION-METADATA-{index}]]");
        if !document.contains(&sentinel) {
            return sentinel;
        }
    }
    unreachable!("a bounded document cannot contain every metadata sentinel")
}

fn last_covered_line(start: Cursor, end: Cursor) -> usize {
    if end.row > start.row && end.col == 0 {
        end.row - 1
    } else {
        end.row
    }
}
