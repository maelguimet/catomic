//! Purpose: retain bounded, render-only change metadata for the latest external reload.
//! Owns: old/new revision comparison, added/changed scalar ranges, and line markers.
//! Must not: mutate buffers, participate in undo, save bytes, or inspect paged/large files.
//! Invariants: metadata describes the exact installed reload buffer; local edits invalidate it.

use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::{Buffer, Cursor};
use crate::file::size::SMALL_FILE_LIMIT_BYTES;

const MAX_DIFF_LINES: usize = 200_000;
const MAX_GRAPHEME_LINE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Changed,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChangedRange {
    pub(crate) start: Cursor,
    pub(crate) end: Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineMarker {
    pub(crate) line: usize,
    pub(crate) kind: ChangeKind,
}

#[derive(Clone, Copy)]
pub(crate) struct VisibleChanges<'a> {
    pub(crate) added_ranges: &'a [ChangedRange],
    pub(crate) changed_ranges: &'a [ChangedRange],
    pub(crate) markers: &'a [LineMarker],
}

#[derive(Debug, Default)]
pub(crate) struct ExternalChanges {
    history_position: Option<u64>,
    added_ranges: Vec<ChangedRange>,
    changed_ranges: Vec<ChangedRange>,
    markers: Vec<LineMarker>,
}

pub(crate) enum DiffOutcome {
    Compared(ExternalChanges),
    Skipped(&'static str),
}

impl DiffOutcome {
    pub(crate) fn into_changes(self) -> ExternalChanges {
        match self {
            Self::Compared(changes) => changes,
            Self::Skipped(_) => ExternalChanges::default(),
        }
    }
}

impl ExternalChanges {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn visible(&self, history_position: u64) -> Option<VisibleChanges<'_>> {
        (self.history_position == Some(history_position) && !self.is_empty()).then_some(
            VisibleChanges {
                added_ranges: &self.added_ranges,
                changed_ranges: &self.changed_ranges,
                markers: &self.markers,
            },
        )
    }

    /// Any local edit invalidates the snapshot-bound set. This deliberately avoids
    /// pretending that coordinates from the reloaded revision still describe a later one.
    pub(crate) fn reconcile(&mut self, history_position: u64) {
        if self
            .history_position
            .is_some_and(|position| position != history_position)
        {
            self.clear();
        }
    }

    fn is_empty(&self) -> bool {
        self.added_ranges.is_empty() && self.changed_ranges.is_empty() && self.markers.is_empty()
    }

    fn finish(mut self, history_position: u64) -> Self {
        self.added_ranges
            .sort_by_key(|range| (range.start.row, range.start.col));
        self.changed_ranges
            .sort_by_key(|range| (range.start.row, range.start.col));
        self.markers.sort_by_key(|marker| marker.line);
        let mut merged = Vec::<LineMarker>::with_capacity(self.markers.len());
        for marker in self.markers.drain(..) {
            if let Some(previous) = merged.last_mut().filter(|item| item.line == marker.line) {
                previous.kind = merge_kind(previous.kind, marker.kind);
            } else {
                merged.push(marker);
            }
        }
        self.markers = merged;
        self.history_position = Some(history_position);
        self
    }

    fn add_range(&mut self, row: usize, start: usize, end: usize, kind: ChangeKind) {
        self.mark_line(row, kind);
        if start >= end {
            return;
        }
        let range = ChangedRange {
            start: Cursor { row, col: start },
            end: Cursor { row, col: end },
        };
        match kind {
            ChangeKind::Added => self.added_ranges.push(range),
            ChangeKind::Changed => self.changed_ranges.push(range),
            ChangeKind::Deleted => {}
        }
    }

    fn mark_line(&mut self, line: usize, kind: ChangeKind) {
        self.markers.push(LineMarker { line, kind });
    }
}

pub(crate) fn compare(old: &dyn Buffer, new: &dyn Buffer) -> DiffOutcome {
    let Some(old_bytes) = old.logical_byte_len() else {
        return DiffOutcome::Skipped("old buffer length is unavailable");
    };
    let Some(new_bytes) = new.logical_byte_len() else {
        return DiffOutcome::Skipped("new buffer length is unavailable");
    };
    if old_bytes as u64 > SMALL_FILE_LIMIT_BYTES || new_bytes as u64 > SMALL_FILE_LIMIT_BYTES {
        return DiffOutcome::Skipped("file exceeds the 10 MiB external-diff limit");
    }
    if old.line_count() > MAX_DIFF_LINES || new.line_count() > MAX_DIFF_LINES {
        return DiffOutcome::Skipped("file exceeds the 200,000-line external-diff limit");
    }

    let old_text = old.to_string();
    let new_text = new.to_string();
    let old_lines: Vec<&str> = old_text.split('\n').collect();
    let new_lines: Vec<&str> = new_text.split('\n').collect();
    let mut changes = ExternalChanges::default();
    for block in changed_blocks(&old_lines, &new_lines) {
        compare_block(&old_lines, &new_lines, block, &mut changes);
    }
    DiffOutcome::Compared(changes.finish(new.edit_history_position()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChangedBlock {
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
}

fn changed_blocks(old: &[&str], new: &[&str]) -> Vec<ChangedBlock> {
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut old_end = old.len();
    let mut new_end = new.len();
    while old_end > prefix && new_end > prefix && old[old_end - 1] == new[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }
    if prefix == old_end && prefix == new_end {
        return Vec::new();
    }

    let anchors = unique_anchors(&old[prefix..old_end], &new[prefix..new_end]);
    let mut blocks = Vec::new();
    let mut old_cursor = prefix;
    let mut new_cursor = prefix;
    for (old_anchor, new_anchor) in anchors {
        let old_anchor = prefix + old_anchor;
        let new_anchor = prefix + new_anchor;
        push_block(&mut blocks, old_cursor, old_anchor, new_cursor, new_anchor);
        old_cursor = old_anchor + 1;
        new_cursor = new_anchor + 1;
    }
    push_block(&mut blocks, old_cursor, old_end, new_cursor, new_end);
    blocks
}

fn push_block(
    blocks: &mut Vec<ChangedBlock>,
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
) {
    if old_start != old_end || new_start != new_end {
        blocks.push(ChangedBlock {
            old_start,
            old_end,
            new_start,
            new_end,
        });
    }
}

fn unique_anchors(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    fn positions<'a>(lines: &[&'a str]) -> HashMap<&'a str, (usize, usize)> {
        let mut positions = HashMap::new();
        for (index, line) in lines.iter().copied().enumerate() {
            positions
                .entry(line)
                .and_modify(|entry: &mut (usize, usize)| entry.1 += 1)
                .or_insert((index, 1));
        }
        positions
    }

    let old_positions = positions(old);
    let new_positions = positions(new);
    let candidates = old_positions
        .iter()
        .filter_map(|(line, &(old_index, old_count))| {
            let &(new_index, new_count) = new_positions.get(line)?;
            (old_count == 1 && new_count == 1).then_some((old_index, new_index))
        })
        .collect::<Vec<_>>();
    longest_increasing_pairs(candidates)
}

fn longest_increasing_pairs(mut pairs: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    pairs.sort_unstable_by_key(|pair| pair.0);
    if pairs.is_empty() {
        return pairs;
    }
    let mut tails: Vec<usize> = Vec::new();
    let mut previous = vec![None; pairs.len()];
    for index in 0..pairs.len() {
        let new_index = pairs[index].1;
        let position = tails
            .binary_search_by(|tail| pairs[*tail].1.cmp(&new_index))
            .unwrap_or_else(|position| position);
        if position > 0 {
            previous[index] = Some(tails[position - 1]);
        }
        if position == tails.len() {
            tails.push(index);
        } else {
            tails[position] = index;
        }
    }
    let mut selected = Vec::with_capacity(tails.len());
    let mut cursor = tails.last().copied();
    while let Some(index) = cursor {
        selected.push(pairs[index]);
        cursor = previous[index];
    }
    selected.reverse();
    selected
}

fn compare_block(
    old_lines: &[&str],
    new_lines: &[&str],
    block: ChangedBlock,
    changes: &mut ExternalChanges,
) {
    let old_count = block.old_end - block.old_start;
    let new_count = block.new_end - block.new_start;
    let paired = old_count.min(new_count);
    for offset in 0..paired {
        compare_line(
            old_lines[block.old_start + offset],
            new_lines[block.new_start + offset],
            block.new_start + offset,
            changes,
        );
    }
    for (offset, line) in new_lines[block.new_start + paired..block.new_end]
        .iter()
        .enumerate()
    {
        let row = block.new_start + paired + offset;
        changes.add_range(row, 0, line.chars().count(), ChangeKind::Added);
    }
    if old_count > paired {
        let marker_line = block
            .new_start
            .saturating_add(paired)
            .min(new_lines.len().saturating_sub(1));
        changes.mark_line(marker_line, ChangeKind::Deleted);
    }
}

fn compare_line(old: &str, new: &str, row: usize, changes: &mut ExternalChanges) {
    if old == new {
        return;
    }
    if old.len() > MAX_GRAPHEME_LINE_BYTES || new.len() > MAX_GRAPHEME_LINE_BYTES {
        changes.add_range(row, 0, new.chars().count(), ChangeKind::Changed);
        return;
    }
    let old_graphemes: Vec<&str> = UnicodeSegmentation::graphemes(old, true).collect();
    let new_graphemes: Vec<&str> = UnicodeSegmentation::graphemes(new, true).collect();
    let mut prefix = 0;
    while prefix < old_graphemes.len()
        && prefix < new_graphemes.len()
        && old_graphemes[prefix] == new_graphemes[prefix]
    {
        prefix += 1;
    }
    let mut old_end = old_graphemes.len();
    let mut new_end = new_graphemes.len();
    while old_end > prefix
        && new_end > prefix
        && old_graphemes[old_end - 1] == new_graphemes[new_end - 1]
    {
        old_end -= 1;
        new_end -= 1;
    }

    let start = new_graphemes[..prefix]
        .iter()
        .map(|grapheme| grapheme.chars().count())
        .sum();
    let end = start
        + new_graphemes[prefix..new_end]
            .iter()
            .map(|grapheme| grapheme.chars().count())
            .sum::<usize>();
    let old_changed = old_end > prefix;
    let new_changed = new_end > prefix;
    match (old_changed, new_changed) {
        (false, true) => changes.add_range(row, start, end, ChangeKind::Added),
        (true, true) => changes.add_range(row, start, end, ChangeKind::Changed),
        (true, false) => changes.mark_line(row, ChangeKind::Deleted),
        (false, false) => {}
    }
}

fn merge_kind(left: ChangeKind, right: ChangeKind) -> ChangeKind {
    if left == right {
        left
    } else {
        ChangeKind::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::PieceTable;

    fn compared(old: &str, new: &str) -> ExternalChanges {
        let old = PieceTable::from_text(old);
        let new = PieceTable::from_text(new);
        match compare(&old, &new) {
            DiffOutcome::Compared(changes) => changes,
            DiffOutcome::Skipped(reason) => panic!("unexpected skip: {reason}"),
        }
    }

    #[test]
    fn insertion_replacement_and_deletion_are_distinct_and_grapheme_safe() {
        let changes = compared("same\ncafé\nremove me\ntail", "same plus\ncafé!\ntail");

        assert!(changes
            .markers
            .iter()
            .any(|marker| marker.kind == ChangeKind::Added));
        assert!(changes
            .markers
            .iter()
            .any(|marker| marker.kind == ChangeKind::Changed));
        assert!(changes
            .markers
            .iter()
            .any(|marker| marker.kind == ChangeKind::Deleted));
        assert!(changes
            .added_ranges
            .iter()
            .all(|range| range.start.col < range.end.col));
        assert!(changes
            .changed_ranges
            .iter()
            .all(|range| range.start.col < range.end.col));
    }

    #[test]
    fn unique_unchanged_lines_split_multiple_external_edits() {
        let changes = compared(
            "one\nanchor-a\ntwo\nanchor-b\nthree",
            "ONE\nanchor-a\nTWO\nanchor-b\nthree",
        );

        assert_eq!(changes.changed_ranges.len(), 2);
        assert_eq!(changes.changed_ranges[0].start.row, 0);
        assert_eq!(changes.changed_ranges[1].start.row, 2);
    }

    #[test]
    fn local_history_change_invalidates_snapshot_coordinates() {
        let mut changes = compared("old", "new");
        assert!(changes.visible(0).is_some());

        changes.reconcile(1);

        assert!(changes.visible(0).is_none());
        assert!(changes.visible(1).is_none());
    }
}
