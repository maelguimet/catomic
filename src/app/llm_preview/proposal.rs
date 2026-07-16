//! Purpose: this file must prepare and apply validated single-buffer LLM proposals.
//! Owns: patch proposals, marked-region targets, synthetic previews, and one edit call.
//! Must not: render UI, create clients, read files, write files, or skip source checks.
//! Invariants: JSON is the only region fallback; application is one Buffer transaction.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io;

use crate::buffer::{Buffer, Cursor};
use crate::llm::patch::Patch;
use crate::llm::replacement;

#[derive(Clone)]
pub(crate) struct RegionTarget {
    start: Cursor,
    end: Cursor,
    original: String,
}

impl RegionTarget {
    pub(crate) fn new(start: Cursor, end: Cursor, original: String) -> Self {
        Self {
            start,
            end,
            original,
        }
    }

    pub(super) fn start(&self) -> Cursor {
        self.start
    }

    pub(super) fn end(&self) -> Cursor {
        self.end
    }

    pub(super) fn original(&self) -> &str {
        &self.original
    }
}

pub(super) enum Proposal {
    Patch(Patch),
    Region(RegionTarget),
}

impl Proposal {
    pub(super) fn apply(
        self,
        buffer: &mut dyn Buffer,
        current: &str,
        proposed_text: &str,
    ) -> io::Result<bool> {
        match self {
            Self::Patch(patch) => {
                if patch.apply_preview(current).ok().as_deref() != Some(proposed_text) {
                    return Ok(false);
                }
                replace_whole_buffer(buffer, proposed_text)
            }
            Self::Region(target) => buffer.replace_range(target.start, target.end, proposed_text),
        }
    }
}

pub(super) fn build_patch(current: &str, text: &str) -> Result<(Proposal, String), String> {
    let patch = Patch::parse(text).map_err(|error| format!("Invalid LLM patch: {error:?}"))?;
    finish_patch(current, patch)
}

pub(super) fn build_patch_for_path(
    current: &str,
    text: &str,
    expected_path: &str,
) -> Result<(Proposal, String), String> {
    let patch = Patch::parse(text).map_err(|error| format!("Invalid LLM patch: {error:?}"))?;
    patch
        .validate_target(expected_path)
        .map_err(|_| format!("LLM patch targets a file other than active path {expected_path}"))?;
    finish_patch(current, patch)
}

fn finish_patch(current: &str, patch: Patch) -> Result<(Proposal, String), String> {
    let proposed_text = patch
        .apply_preview(current)
        .map_err(|error| format!("LLM patch does not match current text: {error:?}"))?;
    Ok((Proposal::Patch(patch), proposed_text))
}

pub(super) fn build_region(
    output: &str,
    target: RegionTarget,
) -> Result<(Proposal, String, String), String> {
    let replacement = replacement::parse(output)
        .map_err(|error| format!("Invalid LLM patch and marked-region replacement: {error:?}"))?;
    let preview = region_preview(&target.original, &replacement);
    Ok((Proposal::Region(target), replacement, preview))
}

fn replace_whole_buffer(buffer: &mut dyn Buffer, text: &str) -> io::Result<bool> {
    let end_row = buffer.line_count().saturating_sub(1);
    let end = Cursor {
        row: end_row,
        col: buffer.line_char_count(end_row).unwrap_or(0),
    };
    buffer.replace_range(Cursor::default(), end, text)
}

fn region_preview(original: &str, replacement: &str) -> String {
    let mut preview =
        String::from("--- selected region\n+++ proposed region\n@@ marked region @@\n");
    for line in original.split_inclusive('\n') {
        preview.push('-');
        preview.push_str(line);
    }
    if !original.is_empty() && !original.ends_with('\n') {
        preview.push('\n');
    }
    for line in replacement.split_inclusive('\n') {
        preview.push('+');
        preview.push_str(line);
    }
    preview
}
