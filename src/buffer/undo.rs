//! Undo / redo stack.
//!
//! Purpose: record piece-level inverse edits for undo/redo without full-text snapshots.
//! Owns: undo and redo vectors of Transactions.
//! Must not: cause recursive recording during apply (caller suppresses via guard);
//!           affect save (save is outside buffer mutation).
//! Invariants:
//! - New edit after undo clears redo stack.
//! - No-op edits produce no Transaction.
//! - Redo of insert re-uses stored piece descriptors (no re-append to add buffer).
//!

use crate::buffer::piece_table::types::Piece;
use crate::buffer::Cursor;

/// The only edit shapes that may extend the current undo transaction.
///
/// Every other semantic edit records an independent transaction.  Keeping this
/// policy here makes it impossible for a range replacement, paste, completion,
/// or newline to accidentally join ordinary typing.
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

/// Piece-level delta: either inserted pieces or deleted pieces at a byte offset.
#[derive(Clone, Debug)]
pub(crate) enum PieceEdit {
    Insert { at: usize, pieces: Vec<Piece> },
    Delete { at: usize, pieces: Vec<Piece> },
}

/// Stack of transactions supporting undo and redo.
/// Tracks a monotonic history position id for exact save-point dirty tracking.
#[derive(Clone, Debug)]
pub struct UndoStack {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Next id to assign on record.
    next_id: u64,
    /// Current position id (0 = initial state before any tx; matches a saved token when equal).
    current_id: u64,
    /// Monotonic content version, including extensions to an active transaction.
    revision: u64,
    /// The transaction that is eligible to receive the next compatible scalar edit.
    active_run: Option<UndoRun>,
    /// Deterministic count of run-owned edit and piece containers.
    /// This structural metric is not an allocator-request counter.
    #[cfg(test)]
    history_container_count: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            current_id: 0,
            revision: 0,
            active_run: None,
            #[cfg(test)]
            history_container_count: 0,
        }
    }
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new transaction. Clears the redo stack (new edit after undo).
    /// Assigns a fresh id and updates current position to it.
    pub(crate) fn record(&mut self, mut tx: Transaction) {
        if tx.edits.is_empty() {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        tx.id = id;
        self.undo.push(tx);
        self.redo.clear();
        self.current_id = id;
        self.active_run = None;
        self.advance_revision();
    }

    /// Record one ordinary typed scalar, extending the active insertion run
    /// only when its add-buffer and document ranges are adjacent.
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
    /// Backspace prepends the newly removed source range; Delete appends it.
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

    fn record_run(&mut self, run: UndoRun, mut tx: Transaction, containers: usize) {
        #[cfg(not(test))]
        let _ = containers;
        let id = self.next_id;
        self.next_id += 1;
        tx.id = id;
        self.undo.push(tx);
        self.redo.clear();
        self.current_id = id;
        self.active_run = Some(run);
        self.advance_revision();
        #[cfg(test)]
        {
            self.history_container_count += containers;
        }
    }

    fn extend_insert(&mut self, at: usize, piece: &Piece, after: CursorState) -> bool {
        let Some(Transaction {
            after: previous_after,
            edits,
            ..
        }) = self.undo.last_mut()
        else {
            return false;
        };
        if previous_after.byte_offset != at {
            return false;
        }
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
        true
    }

    fn extend_delete(
        &mut self,
        run: UndoRun,
        at: usize,
        pieces: &[Piece],
        after: CursorState,
    ) -> bool {
        let Some(Transaction {
            after: previous_after,
            edits,
            ..
        }) = self.undo.last_mut()
        else {
            return false;
        };
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
        let compatible = match run {
            UndoRun::Backspace => at + removed_len == *previous_at,
            UndoRun::DeleteForward => at == *previous_at,
            UndoRun::Typing => false,
        };
        if !compatible {
            return false;
        }
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
        true
    }

    /// Pop the top undo transaction (caller will apply inverse and push to redo).
    /// Updates current position to the id of the now-top undo tx (or 0).
    pub(crate) fn pop_undo(&mut self) -> Option<Transaction> {
        self.finish_run();
        let tx = self.undo.pop()?;
        self.current_id = self.undo.last().map(|t| t.id).unwrap_or(0);
        self.advance_revision();
        Some(tx)
    }

    /// Pop the top redo transaction (caller will apply forward and push to undo).
    pub(crate) fn pop_redo(&mut self) -> Option<Transaction> {
        self.finish_run();
        let tx = self.redo.pop()?;
        self.advance_revision();
        Some(tx)
    }

    /// Push a transaction onto redo (used after successful undo apply).
    pub(crate) fn push_redo(&mut self, tx: Transaction) {
        self.finish_run();
        self.redo.push(tx);
    }

    /// Push a transaction onto undo (used after successful redo apply).
    /// Updates current position to the reapplied tx id.
    pub(crate) fn push_undo(&mut self, tx: Transaction) {
        self.finish_run();
        self.undo.push(tx);
        self.current_id = self.undo.last().map(|t| t.id).unwrap_or(0);
    }

    /// Current edit history position token. Equal tokens mean same point in history
    /// (for save-point dirty computation). No content comparison.
    pub(crate) fn current_history_position(&self) -> u64 {
        self.current_id
    }

    pub(crate) fn content_revision(&self) -> u64 {
        self.revision
    }

    fn advance_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn refresh_active_transaction(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.undo
            .last_mut()
            .expect("an active run has an undo transaction")
            .id = id;
        self.current_id = id;
        self.advance_revision();
    }

    pub(crate) fn has_history(&self) -> bool {
        !self.undo.is_empty() || !self.redo.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn perf_stats(&self) -> (usize, usize) {
        (
            self.undo.len() + self.redo.len(),
            self.retained_history_bytes(),
        )
    }

    pub(crate) fn undo_transaction_count(&self) -> usize {
        self.undo.len()
    }

    #[cfg(test)]
    pub(crate) fn history_container_count(&self) -> usize {
        self.history_container_count
    }

    #[cfg(test)]
    pub(crate) fn retained_history_bytes(&self) -> usize {
        let mut retained_bytes = self.undo.capacity() * std::mem::size_of::<Transaction>()
            + self.redo.capacity() * std::mem::size_of::<Transaction>();
        for transaction in self.undo.iter().chain(&self.redo) {
            retained_bytes += transaction.edits.capacity() * std::mem::size_of::<PieceEdit>();
            for edit in &transaction.edits {
                let pieces = match edit {
                    PieceEdit::Insert { pieces, .. } | PieceEdit::Delete { pieces, .. } => pieces,
                };
                retained_bytes += pieces.capacity() * std::mem::size_of::<Piece>();
            }
        }
        retained_bytes
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
