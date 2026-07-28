//! Undo / redo stack.
//!
//! Purpose: record piece-level inverse edits for undo/redo without full-text snapshots.
//! Owns: bounded undo and redo queues, exact history-position tokens, and retained-size
//! accounting used to threshold add-buffer reclamation.
//! Must not: cause recursive recording during apply (caller suppresses via guard);
//!           affect save (save is outside buffer mutation).
//! Invariants:
//! - New edit after undo clears the complete redo branch.
//! - No-op edits produce no Transaction.
//! - Redo of insert re-uses stored piece descriptors (no re-append to add buffer).
//! - The newest transaction remains usable even when it alone exceeds a budget.
//! - Active grouped runs are the newest transaction and are never partially pruned.
//! - `base_id` identifies the state before the oldest retained undo transaction.

use std::collections::VecDeque;
use std::mem::size_of;
use std::ops::{Deref, DerefMut};

use crate::buffer::piece_table::types::{Piece, Source};
use crate::buffer::Cursor;

pub(crate) const DEFAULT_UNDO_MAX_TRANSACTIONS: usize = 10_000;
pub(crate) const DEFAULT_UNDO_MAX_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const ADD_COMPACTION_MIN_RECLAIM_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct UndoRetentionPolicy {
    pub(crate) max_transactions: usize,
    pub(crate) max_bytes: usize,
}

impl Default for UndoRetentionPolicy {
    fn default() -> Self {
        Self {
            max_transactions: DEFAULT_UNDO_MAX_TRANSACTIONS,
            max_bytes: DEFAULT_UNDO_MAX_BYTES,
        }
    }
}

/// The only edit shapes that may extend the current undo transaction.
///
/// Every other semantic edit records an independent transaction. Keeping this
/// policy here prevents a range replacement, paste, completion, or newline from
/// accidentally joining ordinary typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UndoRun {
    Typing,
    Backspace,
    DeleteForward,
}

/// Cursor snapshot for a transaction boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct CursorState {
    pub(crate) cursor: Cursor,
    pub(crate) byte_offset: usize,
}

/// A recorded edit transaction (forward direction).
/// edits describe what was done; inverse is applied for undo.
/// id is assigned at record time; identifies the state *after* this tx.
#[derive(Clone, Debug)]
pub(crate) struct Transaction {
    pub(crate) before: CursorState,
    pub(crate) after: CursorState,
    pub(crate) edits: Vec<PieceEdit>,
    /// Unique id of the state after this transaction (0 is reserved for initial).
    pub(crate) id: u64,
}

impl Transaction {
    fn retention_metrics(&self) -> (usize, usize, usize) {
        self.edits.iter().fold(
            (size_of::<RecordedTransaction>(), 0usize, 0usize),
            |(retained, add, visits), edit| {
                let pieces = edit.pieces();
                let retained = retained
                    .saturating_add(size_of::<PieceEdit>())
                    .saturating_add(pieces.len().saturating_mul(size_of::<Piece>()))
                    .saturating_add(
                        pieces
                            .iter()
                            .fold(0usize, |bytes, piece| bytes.saturating_add(piece.len)),
                    );
                let add = pieces
                    .iter()
                    .filter(|piece| piece.source == Source::Add)
                    .fold(add, |bytes, piece| bytes.saturating_add(piece.len));
                (retained, add, visits.saturating_add(pieces.len()))
            },
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedTransaction {
    transaction: Transaction,
    retained_bytes: usize,
    add_bytes: usize,
}

impl RecordedTransaction {
    fn new(transaction: Transaction) -> (Self, usize) {
        let (retained_bytes, add_bytes, piece_visits) = transaction.retention_metrics();
        (
            Self {
                transaction,
                retained_bytes,
                add_bytes,
            },
            piece_visits,
        )
    }
}

impl Deref for RecordedTransaction {
    type Target = Transaction;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl DerefMut for RecordedTransaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.transaction
    }
}

/// Piece-level delta: either inserted pieces or deleted pieces at a byte offset.
#[derive(Clone, Debug)]
pub(crate) enum PieceEdit {
    Insert { at: usize, pieces: Vec<Piece> },
    Delete { at: usize, pieces: Vec<Piece> },
}

impl PieceEdit {
    fn pieces(&self) -> &[Piece] {
        match self {
            Self::Insert { pieces, .. } | Self::Delete { pieces, .. } => pieces,
        }
    }

    fn pieces_mut(&mut self) -> &mut Vec<Piece> {
        match self {
            Self::Insert { pieces, .. } | Self::Delete { pieces, .. } => pieces,
        }
    }
}

/// Stack of transactions supporting undo and redo.
/// Tracks a monotonic history position id for exact save-point dirty tracking.
#[derive(Clone, Debug)]
pub struct UndoStack {
    undo: VecDeque<RecordedTransaction>,
    redo: VecDeque<RecordedTransaction>,
    /// State immediately before the oldest retained undo transaction.
    base_id: u64,
    /// Next id to assign on record.
    next_id: u64,
    /// Current position id (0 = initial state before any transaction).
    current_id: u64,
    /// Monotonic content version, including extensions to an active transaction.
    revision: u64,
    /// The newest transaction, when eligible for compatible scalar extension.
    active_run: Option<UndoRun>,
    retained_bytes: usize,
    discarded_add_bytes: usize,
    automatic_retention: Option<UndoRetentionPolicy>,
    /// Deterministic count of run-owned edit and piece containers.
    /// This structural metric is not an allocator-request counter.
    #[cfg(test)]
    history_container_count: usize,
    /// Piece descriptors visited while updating cached retention weights.
    #[cfg(test)]
    retention_piece_visits: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            base_id: 0,
            next_id: 1,
            current_id: 0,
            revision: 0,
            active_run: None,
            retained_bytes: 0,
            discarded_add_bytes: 0,
            automatic_retention: Some(UndoRetentionPolicy::default()),
            #[cfg(test)]
            history_container_count: 0,
            #[cfg(test)]
            retention_piece_visits: 0,
        }
    }
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one independent transaction and clear the redo branch.
    pub(crate) fn record(&mut self, mut transaction: Transaction) {
        if transaction.edits.is_empty() {
            return;
        }
        self.finish_run();
        self.clear_redo();
        let id = self.next_id;
        self.next_id += 1;
        transaction.id = id;
        let (transaction, piece_visits) = RecordedTransaction::new(transaction);
        #[cfg(not(test))]
        let _ = piece_visits;
        self.retained_bytes = self
            .retained_bytes
            .saturating_add(transaction.retained_bytes);
        self.undo.push_back(transaction);
        #[cfg(test)]
        {
            self.retention_piece_visits = self.retention_piece_visits.saturating_add(piece_visits);
        }
        self.current_id = id;
        self.advance_revision();
        self.prune_automatically();
    }

    /// Record one ordinary typed scalar, extending only an adjacent active run.
    pub(crate) fn record_typing_insert(
        &mut self,
        before: CursorState,
        after: CursorState,
        at: usize,
        piece: Piece,
    ) {
        if self.active_run == Some(UndoRun::Typing) && self.extend_insert(at, &piece, after) {
            self.refresh_active_transaction();
            return;
        }
        self.record_run(
            UndoRun::Typing,
            Transaction {
                before,
                after,
                edits: vec![PieceEdit::Insert {
                    at,
                    pieces: vec![piece],
                }],
                id: 0,
            },
            2,
        );
    }

    /// Record one scalar deletion, extending only the matching direction.
    /// Backspace prepends newly removed ranges; Delete appends them.
    pub(crate) fn record_delete(
        &mut self,
        run: UndoRun,
        before: CursorState,
        after: CursorState,
        at: usize,
        pieces: Vec<Piece>,
    ) {
        debug_assert!(matches!(run, UndoRun::Backspace | UndoRun::DeleteForward));
        if self.active_run == Some(run) && self.extend_delete(run, at, &pieces, after) {
            self.refresh_active_transaction();
            return;
        }
        self.record_run(
            run,
            Transaction {
                before,
                after,
                edits: vec![PieceEdit::Delete { at, pieces }],
                id: 0,
            },
            1,
        );
    }

    /// End a coalescing run without altering history or its dirty token.
    pub(crate) fn finish_run(&mut self) {
        self.active_run = None;
    }

    fn record_run(&mut self, run: UndoRun, mut transaction: Transaction, containers: usize) {
        #[cfg(not(test))]
        let _ = containers;
        self.clear_redo();
        let id = self.next_id;
        self.next_id += 1;
        transaction.id = id;
        let (transaction, piece_visits) = RecordedTransaction::new(transaction);
        #[cfg(not(test))]
        let _ = piece_visits;
        self.retained_bytes = self
            .retained_bytes
            .saturating_add(transaction.retained_bytes);
        self.undo.push_back(transaction);
        self.current_id = id;
        self.active_run = Some(run);
        self.advance_revision();
        #[cfg(test)]
        {
            self.history_container_count += containers;
            self.retention_piece_visits = self.retention_piece_visits.saturating_add(piece_visits);
        }
        self.prune_automatically();
    }

    fn extend_insert(&mut self, at: usize, piece: &Piece, after: CursorState) -> bool {
        let Some(transaction) = self.undo.back_mut() else {
            return false;
        };
        let Transaction {
            after: previous_after,
            edits,
            ..
        } = &mut transaction.transaction;
        if previous_after.byte_offset != at {
            return false;
        };
        let [PieceEdit::Insert {
            at: previous_at,
            pieces,
        }] = edits.as_mut_slice()
        else {
            return false;
        };
        let [previous_piece] = pieces.as_mut_slice() else {
            return false;
        };
        if *previous_at + previous_piece.len != at
            || previous_piece.source != piece.source
            || previous_piece.start + previous_piece.len != piece.start
        {
            return false;
        }
        previous_piece.len += piece.len;
        *previous_after = after;
        transaction.retained_bytes = transaction.retained_bytes.saturating_add(piece.len);
        if piece.source == Source::Add {
            transaction.add_bytes = transaction.add_bytes.saturating_add(piece.len);
        }
        self.retained_bytes = self.retained_bytes.saturating_add(piece.len);
        #[cfg(test)]
        {
            self.retention_piece_visits = self.retention_piece_visits.saturating_add(1);
        }
        true
    }

    fn extend_delete(
        &mut self,
        run: UndoRun,
        at: usize,
        pieces: &[Piece],
        after: CursorState,
    ) -> bool {
        let Some(transaction) = self.undo.back_mut() else {
            return false;
        };
        let Transaction {
            after: previous_after,
            edits,
            ..
        } = &mut transaction.transaction;
        let [PieceEdit::Delete {
            at: previous_at,
            pieces: previous_pieces,
        }] = edits.as_mut_slice()
        else {
            return false;
        };
        if pieces.is_empty() {
            return false;
        }
        let removed_len = pieces.iter().map(|piece| piece.len).sum::<usize>();
        let add_bytes = pieces
            .iter()
            .filter(|piece| piece.source == Source::Add)
            .map(|piece| piece.len)
            .sum::<usize>();
        let compatible = match run {
            UndoRun::Backspace => at + removed_len == *previous_at,
            UndoRun::DeleteForward => at == *previous_at,
            UndoRun::Typing => false,
        };
        if !compatible {
            return false;
        }
        let pieces_before = previous_pieces.len();
        match run {
            UndoRun::Backspace => {
                let incoming_len = pieces.len();
                previous_pieces.reserve(incoming_len);
                previous_pieces.extend_from_slice(pieces);
                previous_pieces.rotate_right(incoming_len);
                merge_history_piece_boundary(previous_pieces, incoming_len);
                *previous_at = at;
            }
            UndoRun::DeleteForward => {
                let boundary = previous_pieces.len();
                previous_pieces.extend_from_slice(pieces);
                merge_history_piece_boundary(previous_pieces, boundary);
            }
            UndoRun::Typing => return false,
        }
        *previous_after = after;
        let descriptor_delta = previous_pieces
            .len()
            .saturating_sub(pieces_before)
            .saturating_mul(size_of::<Piece>());
        let retained_delta = removed_len.saturating_add(descriptor_delta);
        transaction.retained_bytes = transaction.retained_bytes.saturating_add(retained_delta);
        transaction.add_bytes = transaction.add_bytes.saturating_add(add_bytes);
        self.retained_bytes = self.retained_bytes.saturating_add(retained_delta);
        #[cfg(test)]
        {
            self.retention_piece_visits = self.retention_piece_visits.saturating_add(pieces.len());
        }
        true
    }

    fn prune_automatically(&mut self) {
        if let Some(policy) = self.automatic_retention {
            self.prune_to(policy);
        }
    }

    fn prune_to(&mut self, policy: UndoRetentionPolicy) {
        while self.undo.len() > 1
            && (self.undo.len() > policy.max_transactions || self.retained_bytes > policy.max_bytes)
        {
            if !self.discard_oldest_undo() {
                break;
            }
        }
    }

    fn discard_oldest_undo(&mut self) -> bool {
        if self.undo.is_empty() || (self.active_run.is_some() && self.undo.len() == 1) {
            return false;
        }
        let transaction = self
            .undo
            .pop_front()
            .expect("non-empty history has an oldest transaction");
        self.base_id = transaction.id;
        self.note_discarded(&transaction);
        true
    }

    fn note_discarded(&mut self, transaction: &RecordedTransaction) {
        self.retained_bytes = self
            .retained_bytes
            .saturating_sub(transaction.retained_bytes);
        self.discarded_add_bytes = self
            .discarded_add_bytes
            .saturating_add(transaction.add_bytes);
    }

    /// Pop the newest undo transaction and move the current token to its parent.
    pub(crate) fn pop_undo(&mut self) -> Option<RecordedTransaction> {
        self.finish_run();
        let transaction = self.undo.pop_back()?;
        self.current_id = self
            .undo
            .back()
            .map_or(self.base_id, |item| item.transaction.id);
        self.advance_revision();
        Some(transaction)
    }

    /// Pop the newest redo transaction.
    pub(crate) fn pop_redo(&mut self) -> Option<RecordedTransaction> {
        self.finish_run();
        let transaction = self.redo.pop_back()?;
        self.advance_revision();
        Some(transaction)
    }

    pub(crate) fn push_redo(&mut self, transaction: RecordedTransaction) {
        self.finish_run();
        self.redo.push_back(transaction);
    }

    pub(crate) fn push_undo(&mut self, transaction: RecordedTransaction) {
        self.finish_run();
        self.current_id = transaction.transaction.id;
        self.undo.push_back(transaction);
    }

    pub(crate) fn clear_redo(&mut self) {
        while let Some(transaction) = self.redo.pop_back() {
            self.note_discarded(&transaction);
        }
    }

    pub(crate) fn discard_oldest_undo_transactions(&mut self, count: usize) {
        for _ in 0..count {
            if !self.discard_oldest_undo() {
                break;
            }
        }
    }

    pub(crate) fn use_external_retention(&mut self) {
        self.automatic_retention = None;
    }

    pub(crate) fn current_history_position(&self) -> u64 {
        self.current_id
    }

    pub(crate) fn content_revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn is_position_retained(&self, position: u64) -> bool {
        position == self.base_id
            || self
                .undo
                .iter()
                .any(|transaction| transaction.transaction.id == position)
            || self
                .redo
                .iter()
                .any(|transaction| transaction.transaction.id == position)
    }

    fn advance_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn refresh_active_transaction(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let transaction = self
            .undo
            .back_mut()
            .expect("an active run has an undo transaction");
        transaction.transaction.id = id;
        self.current_id = id;
        self.advance_revision();
        self.prune_automatically();
    }

    pub(crate) fn has_history(&self) -> bool {
        !self.undo.is_empty() || !self.redo.is_empty()
    }

    pub(crate) fn undo_transaction_count(&self) -> usize {
        self.undo.len()
    }

    pub(crate) fn latest_undo_retained_bytes(&self) -> Option<usize> {
        self.undo
            .back()
            .map(|transaction| transaction.retained_bytes)
    }

    pub(crate) fn discarded_add_bytes(&self) -> usize {
        self.discarded_add_bytes
    }

    pub(crate) fn reset_discarded_add_bytes(&mut self) {
        self.discarded_add_bytes = 0;
    }

    pub(crate) fn for_each_piece(&self, mut visit: impl FnMut(&Piece)) {
        for transaction in self.undo.iter().chain(self.redo.iter()) {
            for edit in &transaction.transaction.edits {
                for piece in edit.pieces() {
                    visit(piece);
                }
            }
        }
    }

    pub(crate) fn for_each_piece_mut(&mut self, mut visit: impl FnMut(&mut Piece)) {
        for transaction in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            for edit in &mut transaction.transaction.edits {
                for piece in edit.pieces_mut() {
                    visit(piece);
                }
            }
        }
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        for transaction in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            transaction.transaction.edits.shrink_to_fit();
            for edit in &mut transaction.transaction.edits {
                edit.pieces_mut().shrink_to_fit();
            }
        }
        self.undo.shrink_to_fit();
        self.redo.shrink_to_fit();
    }

    #[cfg(test)]
    pub(crate) fn perf_stats(&self) -> (usize, usize) {
        (
            self.undo.len() + self.redo.len(),
            self.retained_history_bytes(),
        )
    }

    #[cfg(test)]
    pub(crate) fn history_container_count(&self) -> usize {
        self.history_container_count
    }

    #[cfg(test)]
    pub(crate) fn reset_retention_piece_visits(&mut self) {
        self.retention_piece_visits = 0;
    }

    #[cfg(test)]
    pub(crate) fn retention_piece_visits(&self) -> usize {
        self.retention_piece_visits
    }

    #[cfg(test)]
    pub(crate) fn retained_history_bytes(&self) -> usize {
        let mut retained_bytes = self.undo.capacity() * size_of::<RecordedTransaction>()
            + self.redo.capacity() * size_of::<RecordedTransaction>();
        for transaction in self.undo.iter().chain(self.redo.iter()) {
            retained_bytes += transaction.transaction.edits.capacity() * size_of::<PieceEdit>();
            for edit in &transaction.transaction.edits {
                let pieces = match edit {
                    PieceEdit::Insert { pieces, .. } | PieceEdit::Delete { pieces, .. } => pieces,
                };
                retained_bytes += pieces.capacity() * size_of::<Piece>();
            }
        }
        retained_bytes
    }

    #[cfg(test)]
    pub(crate) fn set_retention_policy(&mut self, policy: UndoRetentionPolicy) {
        self.automatic_retention = Some(policy);
        self.prune_to(policy);
    }

    #[cfg(test)]
    pub(crate) fn retained_transaction_count(&self) -> usize {
        self.undo.len() + self.redo.len()
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

fn merge_history_piece_boundary(pieces: &mut Vec<Piece>, right_index: usize) {
    if right_index == 0 || right_index >= pieces.len() {
        return;
    }
    let left = pieces[right_index - 1];
    let right = pieces[right_index];
    if left.source == right.source && left.start + left.len == right.start {
        pieces[right_index - 1].len += right.len;
        pieces.remove(right_index);
    }
}
