//! Undo / redo stack (Phase 1C).
//!
//! Purpose: record piece-level inverse edits for undo/redo without full-text snapshots.
//! Owns: undo and redo vectors of Transactions.
//! Must not: cause recursive recording during apply (caller suppresses via guard);
//!           affect save (save is outside buffer mutation).
//! Invariants:
//! - New edit after undo clears redo stack.
//! - No-op edits produce no Transaction.
//! - Redo of insert re-uses stored piece descriptors (no re-append to add buffer).
//! Phase: 1C

use crate::buffer::Cursor;
use crate::buffer::piece_table::types::Piece;

/// Cursor snapshot for a transaction boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct CursorState {
    pub(crate) cursor: Cursor,
    pub(crate) byte_offset: usize,
}

/// A recorded edit transaction (forward direction).
/// edits describe what was done; inverse is applied for undo.
#[derive(Clone, Debug)]
pub(crate) struct Transaction {
    pub(crate) before: CursorState,
    pub(crate) after: CursorState,
    pub(crate) edits: Vec<PieceEdit>,
}

/// Piece-level delta: either inserted pieces or deleted pieces at a byte offset.
#[derive(Clone, Debug)]
pub(crate) enum PieceEdit {
    Insert {
        at: usize,
        pieces: Vec<Piece>,
    },
    Delete {
        at: usize,
        pieces: Vec<Piece>,
    },
}

/// Stack of transactions supporting undo and redo.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new transaction. Clears the redo stack (new edit after undo).
    pub(crate) fn record(&mut self, tx: Transaction) {
        if tx.edits.is_empty() {
            return;
        }
        self.undo.push(tx);
        self.redo.clear();
    }

    /// Pop the top undo transaction (caller will apply inverse and push to redo).
    pub(crate) fn pop_undo(&mut self) -> Option<Transaction> {
        self.undo.pop()
    }

    /// Pop the top redo transaction (caller will apply forward and push to undo).
    pub(crate) fn pop_redo(&mut self) -> Option<Transaction> {
        self.redo.pop()
    }

    /// Push a transaction onto redo (used after successful undo apply).
    pub(crate) fn push_redo(&mut self, tx: Transaction) {
        self.redo.push(tx);
    }

    /// Push a transaction onto undo (used after successful redo apply).
    pub(crate) fn push_undo(&mut self, tx: Transaction) {
        self.undo.push(tx);
    }

    #[cfg(test)]
    pub(crate) fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[cfg(test)]
    pub(crate) fn redo_len(&self) -> usize {
        self.redo.len()
    }
}
