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

use crate::buffer::piece_table::types::Piece;
use crate::buffer::Cursor;

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
/// Phase 2-j: tracks a monotonic history position id for exact save-point dirty tracking.
#[derive(Clone, Debug)]
pub struct UndoStack {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Next id to assign on record.
    next_id: u64,
    /// Current position id (0 = initial state before any tx; matches a saved token when equal).
    current_id: u64,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            current_id: 0,
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
    }

    /// Pop the top undo transaction (caller will apply inverse and push to redo).
    /// Updates current position to the id of the now-top undo tx (or 0).
    pub(crate) fn pop_undo(&mut self) -> Option<Transaction> {
        let tx = self.undo.pop()?;
        self.current_id = self.undo.last().map(|t| t.id).unwrap_or(0);
        Some(tx)
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
    /// Updates current position to the reapplied tx id.
    pub(crate) fn push_undo(&mut self, tx: Transaction) {
        self.undo.push(tx);
        self.current_id = self.undo.last().map(|t| t.id).unwrap_or(0);
    }

    /// Current edit history position token. Equal tokens mean same point in history
    /// (for save-point dirty computation). No content comparison.
    pub(crate) fn current_history_position(&self) -> u64 {
        self.current_id
    }
}
