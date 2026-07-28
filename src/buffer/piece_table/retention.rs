//! Bounded PieceTable history and add-buffer reclamation.
//!
//! Purpose: centralize transaction recording, page-coordination seams, and rare
//! descriptor rebasing after discarded history makes add ranges unreachable.
//! Must not: materialize original backing, scan the logical document, or alter
//! cursor/index state.

use std::ops::Range;

#[cfg(test)]
use crate::buffer::undo::UndoRetentionPolicy;
use crate::buffer::undo::{CursorState, Transaction, UndoRun, ADD_COMPACTION_MIN_RECLAIM_BYTES};

use super::scalar_index::ScalarIndex;
use super::types::{Piece, PieceTable, Source};

impl PieceTable {
    pub(super) fn record_transaction(&mut self, transaction: Transaction) {
        self.undo_stack.record(transaction);
        self.maybe_compact_add_buffer();
    }

    pub(super) fn record_typing_history(
        &mut self,
        before: CursorState,
        after: CursorState,
        at: usize,
        piece: Piece,
    ) {
        self.undo_stack
            .record_typing_insert(before, after, at, piece);
        self.maybe_compact_add_buffer();
    }

    pub(super) fn record_delete_history(
        &mut self,
        run: UndoRun,
        before: CursorState,
        after: CursorState,
        at: usize,
        pieces: Vec<Piece>,
    ) {
        self.undo_stack
            .record_delete(run, before, after, at, pieces);
        self.maybe_compact_add_buffer();
    }

    pub(crate) fn use_external_history_retention(&mut self) {
        self.undo_stack.use_external_retention();
    }

    pub(crate) fn latest_undo_retained_bytes(&self) -> Option<usize> {
        self.undo_stack.latest_undo_retained_bytes()
    }

    pub(crate) fn clear_redo_history(&mut self) {
        self.undo_stack.clear_redo();
        self.maybe_compact_add_buffer();
    }

    pub(crate) fn discard_oldest_undo_transactions(&mut self, count: usize) {
        self.undo_stack.discard_oldest_undo_transactions(count);
        self.maybe_compact_add_buffer();
    }

    pub(crate) fn history_position_is_retained(&self, position: u64) -> bool {
        self.undo_stack.is_position_retained(position)
    }

    fn maybe_compact_add_buffer(&mut self) {
        if self.undo_stack.discarded_add_bytes() < ADD_COMPACTION_MIN_RECLAIM_BYTES
            || self.add.len() < ADD_COMPACTION_MIN_RECLAIM_BYTES
        {
            return;
        }
        self.compact_add_buffer(ADD_COMPACTION_MIN_RECLAIM_BYTES);
    }

    fn compact_add_buffer(&mut self, minimum_reclaim: usize) -> usize {
        let ranges = self.reachable_add_ranges();
        let reachable_bytes = ranges
            .iter()
            .fold(0usize, |total, range| total.saturating_add(range.len()));
        let reclaimable = self.add.len().saturating_sub(reachable_bytes);
        self.undo_stack.reset_discarded_add_bytes();

        // Avoid copying a large live add store for a marginal return. The
        // minimum threshold also prevents ordinary typing from rebuilding.
        if reclaimable < minimum_reclaim || reclaimable < self.add.len().saturating_add(3) / 4 {
            return 0;
        }

        // Leave a small fraction of the reclamation threshold as append
        // headroom so the first post-compaction edit does not immediately
        // double a large live allocation.
        let append_headroom = if reachable_bytes == 0 {
            0
        } else {
            minimum_reclaim / 2
        };
        let target_capacity = reachable_bytes
            .checked_add(append_headroom)
            .unwrap_or(reachable_bytes);
        let mut rebased = String::with_capacity(target_capacity);
        let mut mappings = Vec::with_capacity(ranges.len());
        for range in ranges {
            let new_start = rebased.len();
            rebased.push_str(&self.add[range.clone()]);
            mappings.push(AddRangeMapping {
                old: range,
                new_start,
            });
        }

        self.pieces
            .for_each_mut(|piece| remap_add_piece(piece, &mappings));
        self.undo_stack
            .for_each_piece_mut(|piece| remap_add_piece(piece, &mappings));
        let rebased_scalars = ScalarIndex::for_appendable_text(&rebased);
        self.add = rebased;
        self.add_scalars = rebased_scalars;
        self.undo_stack.shrink_to_fit();
        reclaimable
    }

    fn reachable_add_ranges(&self) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        #[cfg(test)]
        let mut descriptor_scans = 0usize;
        self.pieces.for_each(|piece| {
            #[cfg(test)]
            {
                descriptor_scans += 1;
            }
            push_add_range(piece, &mut ranges);
        });
        self.undo_stack.for_each_piece(|piece| {
            #[cfg(test)]
            {
                descriptor_scans += 1;
            }
            push_add_range(piece, &mut ranges);
        });
        #[cfg(test)]
        self.compaction_descriptor_scans.set(
            self.compaction_descriptor_scans
                .get()
                .saturating_add(descriptor_scans),
        );
        ranges.sort_unstable_by_key(|range| (range.start, range.end));

        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = merged.last_mut() {
                if range.start <= previous.end {
                    previous.end = previous.end.max(range.end);
                    continue;
                }
            }
            merged.push(range);
        }
        merged
    }

    #[cfg(test)]
    pub(crate) fn set_history_retention_for_test(
        &mut self,
        max_transactions: usize,
        max_bytes: usize,
    ) {
        self.undo_stack.set_retention_policy(UndoRetentionPolicy {
            max_transactions,
            max_bytes,
        });
        self.maybe_compact_add_buffer();
    }

    #[cfg(test)]
    pub(crate) fn compact_add_buffer_for_test(&mut self) -> usize {
        self.compact_add_buffer(0)
    }

    #[cfg(test)]
    pub(crate) fn retained_history_transactions_for_test(&self) -> usize {
        self.undo_stack.retained_transaction_count()
    }

    #[cfg(test)]
    pub(crate) fn retained_history_bytes_for_test(&self) -> usize {
        self.undo_stack.retained_bytes()
    }

    #[cfg(test)]
    pub(crate) fn add_storage_for_test(&self) -> (usize, usize) {
        (self.add.len(), self.add.capacity())
    }

    #[cfg(test)]
    pub(crate) fn reset_retention_piece_visits_for_test(&mut self) {
        self.undo_stack.reset_retention_piece_visits();
    }

    #[cfg(test)]
    pub(crate) fn retention_piece_visits_for_test(&self) -> usize {
        self.undo_stack.retention_piece_visits()
    }

    #[cfg(test)]
    pub(crate) fn reset_compaction_descriptor_scans_for_test(&self) {
        self.compaction_descriptor_scans.set(0);
    }

    #[cfg(test)]
    pub(crate) fn compaction_descriptor_scans_for_test(&self) -> usize {
        self.compaction_descriptor_scans.get()
    }
}

struct AddRangeMapping {
    old: Range<usize>,
    new_start: usize,
}

fn push_add_range(piece: &Piece, ranges: &mut Vec<Range<usize>>) {
    if piece.source == Source::Add && piece.len > 0 {
        ranges.push(piece.start..piece.start + piece.len);
    }
}

fn remap_add_piece(piece: &mut Piece, mappings: &[AddRangeMapping]) {
    if piece.source != Source::Add || piece.len == 0 {
        return;
    }
    let mapping_index = mappings.partition_point(|mapping| mapping.old.start <= piece.start);
    let mapping = &mappings[mapping_index.saturating_sub(1)];
    debug_assert!(piece.start + piece.len <= mapping.old.end);
    piece.start = mapping.new_start + piece.start - mapping.old.start;
}
