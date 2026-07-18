//! Purpose: parse, display, revalidate, and explicitly apply inline-clanker proposals.
//! Owns: strict response formats, exact before/after preview, atomic edits, and queue advance.
//! Must not: construct unconfirmed clients, write files, hide cleanup, or edit outside targets.
//! Invariants: Enter alone applies; combined edits and cleanup share one buffer transaction.
//! Phase: issue #65 one-key inline clanker workflow.

use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::buffer::{Cursor, TextEdit};
use crate::llm::inline::InlineScope;
use crate::llm::patch::{Patch, PatchVisualization};

use super::{ChangeSet, ChangedRange, PendingEdit, PreparedWorkflow};

pub(super) fn open_response(
    app: &mut super::super::App,
    out: &mut dyn Write,
    prepared: PreparedWorkflow,
    output: &str,
) -> io::Result<()> {
    let result = match prepared.draft.scope {
        InlineScope::FullFile => full_file_proposal(app, &prepared, output),
        InlineScope::Selection | InlineScope::Blocks => region_proposal(&prepared, output),
    };
    let (mut edits, applied_changes) = match result {
        Ok(proposal) => proposal,
        Err(error) => return super::request::fail_or_continue(app, out, prepared, &error),
    };
    add_cleanup_if_final(&prepared, &mut edits);
    edits.sort_by(|left, right| {
        (right.edit.start.row, right.edit.start.col)
            .cmp(&(left.edit.start.row, left.edit.start.col))
    });
    let applied_changes = finalize_change_positions(applied_changes, &edits);
    let (preview_text, preview_changes) = build_preview(&prepared, &edits, output);
    super::preview_ui::open(
        app,
        out,
        prepared,
        edits,
        applied_changes,
        preview_text,
        preview_changes,
    )
}

fn region_proposal(
    prepared: &PreparedWorkflow,
    output: &str,
) -> Result<(Vec<PendingEdit>, ChangeSet), String> {
    let unit = &prepared.draft.requests[prepared.request_index];
    let replacements = if unit.target_ids.len() == 1 {
        let replacement = crate::llm::replacement::parse(output)
            .map_err(|error| format!("malformed scoped replacement: {error:?}"))?;
        BTreeMap::from([(unit.target_ids[0], replacement)])
    } else {
        crate::llm::inline::parse_combined_replacements(output, &unit.target_ids)
            .map_err(|error| format!("malformed multi-block replacement: {error:?}"))?
    };
    let mut edits = Vec::new();
    let mut changes = ChangeSet::default();
    for (&id, replacement) in &replacements {
        let target = prepared
            .draft
            .target(id)
            .ok_or_else(|| format!("response names missing request-local block {id}"))?;
        if replacement == &target.range.original {
            continue;
        }
        let edit = TextEdit {
            start: target.range.start,
            end: target.range.end,
            replacement: replacement.clone(),
        };
        if replacement.is_empty() {
            changes.gutter_lines.push(edit.start.row);
        } else {
            changes.ranges.push(ChangedRange {
                start: edit.start,
                end: super::changes::cursor_after_text(edit.start, replacement),
            });
        }
        edits.push(PendingEdit {
            target_id: Some(id),
            edit,
            original: target.range.original.clone(),
            label: match prepared.draft.scope {
                InlineScope::Selection => format!(
                    "selection lines {}-{}",
                    target.range.first_line + 1,
                    target.range.last_line + 1
                ),
                _ => format!(
                    "context block {id} lines {}-{}",
                    target.range.first_line + 1,
                    target.range.last_line + 1
                ),
            },
        });
    }
    if edits.is_empty() && !cleanup_is_due(prepared) {
        return Err("proposal makes no scoped change".to_string());
    }
    Ok((edits, changes))
}

fn full_file_proposal(
    app: &super::super::App,
    prepared: &PreparedWorkflow,
    output: &str,
) -> Result<(Vec<PendingEdit>, ChangeSet), String> {
    let source = &prepared.draft.requests[0].text;
    let sentinel = prepared
        .draft
        .full_file_sentinel
        .as_deref()
        .ok_or_else(|| "full-file request lost its control sentinel".to_string())?;
    if output
        .lines()
        .any(|line| line == format!("-{sentinel}") || line == format!("+{sentinel}"))
    {
        return Err("full-file patch attempts to edit instruction metadata".to_string());
    }
    let patch =
        Patch::parse(output).map_err(|error| format!("invalid full-file patch: {error:?}"))?;
    patch.validate_target(&prepared.path).map_err(|_| {
        format!(
            "full-file patch targets a path other than {}",
            prepared.path
        )
    })?;
    let proposed = patch
        .apply_preview(source)
        .map_err(|error| format!("full-file patch does not match confirmed context: {error:?}"))?;
    if proposed.match_indices(sentinel).count() != 1 {
        return Err("full-file patch changed or duplicated instruction metadata".to_string());
    }
    let sentinel_line = proposed[..proposed.find(sentinel).expect("one sentinel")]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    let final_text = restore_or_remove_metadata(&proposed, sentinel, prepared)?;
    let current = app.buffer.to_string();
    if final_text == current {
        return Err("proposal makes no full-file change".to_string());
    }
    let changes = map_patch_changes(patch.visualization(), sentinel_line, prepared, &final_text);
    let end_row = app.buffer.line_count().saturating_sub(1);
    let edit = TextEdit {
        start: Cursor::default(),
        end: Cursor {
            row: end_row,
            col: app.buffer.line_char_count(end_row).unwrap_or(0),
        },
        replacement: final_text,
    };
    Ok((
        vec![PendingEdit {
            target_id: None,
            edit,
            original: current,
            label: format!("full-file unified patch for {}", prepared.path),
        }],
        changes,
    ))
}

fn restore_or_remove_metadata(
    proposed: &str,
    sentinel: &str,
    prepared: &PreparedWorkflow,
) -> Result<String, String> {
    let at = proposed.find(sentinel).expect("validated sentinel");
    let metadata = &prepared.draft.instruction;
    let (start, end, replacement) = if !prepared.inline.remove_instruction_after_apply {
        (at, at + sentinel.len(), metadata.metadata.original.as_str())
    } else if metadata.cleanup.start != metadata.metadata.start {
        let start = at.checked_sub(1).ok_or_else(|| {
            "instruction cleanup expected a preceding newline that is missing".to_string()
        })?;
        if proposed.as_bytes()[start] != b'\n' {
            return Err("instruction cleanup preceding newline drifted".to_string());
        }
        (start, at + sentinel.len(), "")
    } else if metadata.cleanup.end != metadata.metadata.end {
        let end = at + sentinel.len();
        if proposed.as_bytes().get(end) != Some(&b'\n') {
            return Err("instruction cleanup terminating newline drifted".to_string());
        }
        (at, end + 1, "")
    } else {
        (at, at + sentinel.len(), "")
    };
    let mut restored = String::with_capacity(proposed.len());
    restored.push_str(&proposed[..start]);
    restored.push_str(replacement);
    restored.push_str(&proposed[end..]);
    Ok(restored)
}

fn map_patch_changes(
    visualization: PatchVisualization,
    sentinel_line: usize,
    prepared: &PreparedWorkflow,
    final_text: &str,
) -> ChangeSet {
    let map_line = |line: usize| -> usize {
        if line <= sentinel_line {
            return line;
        }
        if prepared.inline.remove_instruction_after_apply {
            line.saturating_sub(1)
        } else {
            let metadata_lines = prepared
                .draft
                .instruction
                .metadata
                .original
                .split('\n')
                .count();
            line.saturating_add(metadata_lines.saturating_sub(1))
        }
    };
    let line_count = final_text.split('\n').count();
    let mut changes = ChangeSet::default();
    for (start, end) in visualization.added_line_ranges {
        let start = map_line(start).min(line_count.saturating_sub(1));
        let end = map_line(end).min(line_count);
        if start < end {
            changes.ranges.push(ChangedRange {
                start: Cursor { row: start, col: 0 },
                end: Cursor { row: end, col: 0 },
            });
        }
    }
    changes.gutter_lines.extend(
        visualization
            .deleted_at_lines
            .into_iter()
            .map(map_line)
            .map(|line| line.min(line_count.saturating_sub(1))),
    );
    if prepared.inline.remove_instruction_after_apply {
        changes
            .gutter_lines
            .push(map_line(sentinel_line).min(line_count.saturating_sub(1)));
    }
    changes
}

fn add_cleanup_if_final(prepared: &PreparedWorkflow, edits: &mut Vec<PendingEdit>) {
    if prepared.draft.scope == InlineScope::FullFile || !cleanup_is_due(prepared) {
        return;
    }
    let cleanup = &prepared.draft.instruction.cleanup;
    edits.push(PendingEdit {
        target_id: None,
        edit: TextEdit {
            start: cleanup.start,
            end: cleanup.end,
            replacement: String::new(),
        },
        original: cleanup.original.clone(),
        label: format!(
            "confirmed instruction cleanup line {}",
            prepared.draft.instruction.display_line
        ),
    });
}

fn cleanup_is_due(prepared: &PreparedWorkflow) -> bool {
    prepared.inline.remove_instruction_after_apply
        && !prepared.had_failure
        && prepared.request_index + 1 == prepared.draft.requests.len()
}

fn finalize_change_positions(mut changes: ChangeSet, edits: &[PendingEdit]) -> ChangeSet {
    if edits.iter().all(|pending| pending.target_id.is_none()) {
        changes.gutter_lines.sort_unstable();
        changes.gutter_lines.dedup();
        return changes;
    }
    let text_edits: Vec<_> = edits.iter().map(|pending| pending.edit.clone()).collect();
    let model_starts: Vec<_> = edits
        .iter()
        .filter(|pending| pending.target_id.is_some())
        .map(|pending| (pending.edit.start, pending.edit.replacement.clone()))
        .collect();
    changes.ranges.clear();
    changes.gutter_lines.clear();
    for (start, replacement) in model_starts {
        let final_start = final_start_after_edits(start, &text_edits);
        if replacement.is_empty() {
            changes.gutter_lines.push(final_start.row);
        } else {
            changes.ranges.push(ChangedRange {
                start: final_start,
                end: super::changes::cursor_after_text(final_start, &replacement),
            });
            changes.gutter_lines.push(final_start.row);
        }
    }
    for pending in edits.iter().filter(|pending| {
        pending.target_id.is_none() && pending.label.contains("instruction cleanup")
    }) {
        changes
            .gutter_lines
            .push(final_start_after_edits(pending.edit.start, &text_edits).row);
    }
    changes.gutter_lines.sort_unstable();
    changes.gutter_lines.dedup();
    changes
}

fn final_start_after_edits(start: Cursor, edits: &[TextEdit]) -> Cursor {
    let mut shifted = start;
    for edit in edits {
        if (edit.end.row, edit.end.col) <= (start.row, start.col) && edit.start != start {
            shifted = shift_after(shifted, edit);
        }
    }
    shifted
}

fn shift_after(cursor: Cursor, edit: &TextEdit) -> Cursor {
    if cursor.row == edit.end.row {
        let end = super::changes::cursor_after_text(edit.start, &edit.replacement);
        Cursor {
            row: end.row,
            col: end.col + cursor.col.saturating_sub(edit.end.col),
        }
    } else {
        let removed = edit.end.row.saturating_sub(edit.start.row);
        let added = edit
            .replacement
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        Cursor {
            row: if added >= removed {
                cursor.row + added - removed
            } else {
                cursor.row.saturating_sub(removed - added)
            },
            col: cursor.col,
        }
    }
}

fn build_preview(
    prepared: &PreparedWorkflow,
    edits: &[PendingEdit],
    model_output: &str,
) -> (String, ChangeSet) {
    let mut text = format!(
        "Inline clanker proposal (read-only)\nModel: {}\nEndpoint: {}\nInstruction line: {}\nCleanup: {}\n\n",
        prepared.preset.model,
        prepared.destination,
        prepared.draft.instruction.display_line,
        if cleanup_is_due(prepared) { "included" } else { "not included" }
    );
    for pending in edits {
        let after =
            super::changes::cursor_after_text(pending.edit.start, &pending.edit.replacement);
        text.push_str(&format!(
            "@@ {}; before {}:{}-{}:{} ({} bytes); after {}:{}-{}:{} ({} bytes) @@\n",
            pending.label,
            pending.edit.start.row + 1,
            pending.edit.start.col + 1,
            pending.edit.end.row + 1,
            pending.edit.end.col + 1,
            pending.original.len(),
            pending.edit.start.row + 1,
            pending.edit.start.col + 1,
            after.row + 1,
            after.col + 1,
            pending.edit.replacement.len(),
        ));
        append_prefixed(&mut text, '-', &pending.original);
        append_prefixed(&mut text, '+', &pending.edit.replacement);
        text.push('\n');
    }
    if prepared.draft.scope == InlineScope::FullFile {
        text.push_str("Validated model unified diff:\n");
        text.push_str(model_output);
        if !model_output.ends_with('\n') {
            text.push('\n');
        }
    }
    let changes = preview_line_changes(&text);
    (text, changes)
}

fn append_prefixed(out: &mut String, prefix: char, value: &str) {
    if value.is_empty() {
        out.push(prefix);
        out.push_str("<empty>\n");
        return;
    }
    for line in value.split_inclusive('\n') {
        out.push(prefix);
        out.push_str(line);
    }
    if !value.ends_with('\n') {
        out.push('\n');
    }
}

fn preview_line_changes(text: &str) -> ChangeSet {
    let mut changes = ChangeSet::default();
    for (row, line) in text.lines().enumerate() {
        if line.starts_with('+') && !line.starts_with("+++") {
            changes.ranges.push(ChangedRange {
                start: Cursor { row, col: 0 },
                end: Cursor {
                    row,
                    col: line.chars().count(),
                },
            });
            changes.gutter_lines.push(row);
        } else if line.starts_with('-') && !line.starts_with("---") {
            changes.gutter_lines.push(row);
        }
    }
    changes
}
