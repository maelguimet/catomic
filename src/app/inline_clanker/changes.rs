//! Purpose: retain render-only inline-clanker ranges across their undo/redo history positions.
//! Owns: bounded change versions, document-coordinate shifting, and visible semantic metadata.
//! Must not: mutate buffers, alter saved bytes, render ANSI, or outlive its owning buffer.
//! Invariants: unknown history positions expose no marks; ranges remain scalar document ranges.

use crate::buffer::{Cursor, TextEdit};
use crate::llm::inline::{CapturedRange, InlineDraft};

use super::ChangeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChangedRange {
    pub(crate) start: Cursor,
    pub(crate) end: Cursor,
}

#[derive(Clone, Copy)]
pub(crate) struct VisibleChanges<'a> {
    pub(crate) ranges: &'a [ChangedRange],
    pub(crate) gutter_lines: &'a [usize],
}

impl<'a> VisibleChanges<'a> {
    pub(super) fn from_set(set: &'a ChangeSet) -> Self {
        Self {
            ranges: &set.ranges,
            gutter_lines: &set.gutter_lines,
        }
    }
}

#[derive(Default)]
pub(crate) struct ChangeHistory {
    versions: Vec<ChangeVersion>,
}

struct ChangeVersion {
    history_position: u64,
    changes: ChangeSet,
}

impl ChangeHistory {
    pub(crate) fn clear(&mut self) {
        self.versions.clear();
    }

    pub(crate) fn visible(&self, history_position: u64) -> Option<VisibleChanges<'_>> {
        self.versions
            .iter()
            .find(|version| version.history_position == history_position)
            .filter(|version| {
                !version.changes.ranges.is_empty() || !version.changes.gutter_lines.is_empty()
            })
            .map(|version| VisibleChanges::from_set(&version.changes))
    }

    pub(super) fn record(
        &mut self,
        first_apply: bool,
        before: u64,
        after: u64,
        changes: ChangeSet,
    ) {
        if first_apply {
            self.versions.clear();
            self.versions.push(ChangeVersion {
                history_position: before,
                changes: ChangeSet::default(),
            });
        }
        self.versions
            .retain(|version| version.history_position != after);
        self.versions.push(ChangeVersion {
            history_position: after,
            changes,
        });
    }

    pub(crate) fn reconcile(&mut self, history_position: u64) {
        if !self
            .versions
            .iter()
            .any(|version| version.history_position == history_position)
        {
            self.clear();
        }
    }

    pub(super) fn changes_at(&self, history_position: u64) -> ChangeSet {
        self.versions
            .iter()
            .find(|version| version.history_position == history_position)
            .map(|version| version.changes.clone())
            .unwrap_or_default()
    }
}

pub(super) fn shift_set(changes: &mut ChangeSet, edits: &[TextEdit]) {
    for edit in edits {
        for range in &mut changes.ranges {
            range.start = shift_cursor(range.start, edit);
            range.end = shift_cursor(range.end, edit);
        }
        for line in &mut changes.gutter_lines {
            let shifted = shift_cursor(Cursor { row: *line, col: 0 }, edit);
            *line = shifted.row;
        }
    }
    changes.gutter_lines.sort_unstable();
    changes.gutter_lines.dedup();
}

pub(super) fn update_draft_after_edit(draft: &mut InlineDraft, target_id: usize, edit: &TextEdit) {
    shift_captured(&mut draft.instruction.metadata, edit);
    shift_captured(&mut draft.instruction.cleanup, edit);
    draft.instruction.display_line = draft.instruction.metadata.first_line + 1;
    for guard in &mut draft.delimiter_guards {
        shift_captured(guard, edit);
    }
    for target in &mut draft.targets {
        if target.id == target_id {
            target.range.end = cursor_after_text(target.range.start, &edit.replacement);
            target.range.original.clone_from(&edit.replacement);
            sync_lines(&mut target.range);
        } else {
            shift_captured(&mut target.range, edit);
        }
    }
}

fn shift_captured(range: &mut CapturedRange, edit: &TextEdit) {
    range.start = shift_cursor(range.start, edit);
    range.end = shift_cursor(range.end, edit);
    sync_lines(range);
}

fn sync_lines(range: &mut CapturedRange) {
    range.first_line = range.start.row;
    range.last_line = if range.end.row > range.start.row && range.end.col == 0 {
        range.end.row - 1
    } else {
        range.end.row
    };
}

pub(super) fn cursor_after_text(start: Cursor, text: &str) -> Cursor {
    let newlines = text.bytes().filter(|byte| *byte == b'\n').count();
    if newlines == 0 {
        Cursor {
            row: start.row,
            col: start.col + text.chars().count(),
        }
    } else {
        Cursor {
            row: start.row + newlines,
            col: text.rsplit('\n').next().unwrap_or_default().chars().count(),
        }
    }
}

fn shift_cursor(cursor: Cursor, edit: &TextEdit) -> Cursor {
    if (cursor.row, cursor.col) < (edit.end.row, edit.end.col) {
        return cursor;
    }
    let replacement_end = cursor_after_text(edit.start, &edit.replacement);
    if cursor.row == edit.end.row {
        return Cursor {
            row: replacement_end.row,
            col: replacement_end
                .col
                .saturating_add(cursor.col.saturating_sub(edit.end.col)),
        };
    }
    let removed_rows = edit.end.row.saturating_sub(edit.start.row);
    let added_rows = replacement_end.row.saturating_sub(edit.start.row);
    Cursor {
        row: if added_rows >= removed_rows {
            cursor.row.saturating_add(added_rows - removed_rows)
        } else {
            cursor.row.saturating_sub(removed_rows - added_rows)
        },
        col: cursor.col,
    }
}
