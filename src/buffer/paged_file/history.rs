//! Purpose: track cross-page edit order without snapshots or content copies.
//! Owns: global undo/redo ordering and exact history-position tokens.
//! Must not: edit pages, read descriptors, render, save, or know App state.
//! Invariants: transaction ids are stable across undo/redo; a new edit clears redo.

#[derive(Clone, Copy)]
pub(super) struct PageTransaction {
    pub(super) page_start: usize,
    id: u64,
}

#[derive(Default)]
pub(super) struct PageHistory {
    undo: Vec<PageTransaction>,
    redo: Vec<PageTransaction>,
    next_id: u64,
    current_id: u64,
}

impl PageHistory {
    pub(super) fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    pub(super) fn record(&mut self, page_start: usize) {
        let transaction = PageTransaction {
            page_start,
            id: self.next_id,
        };
        self.next_id += 1;
        self.current_id = transaction.id;
        self.undo.push(transaction);
        self.redo.clear();
    }

    pub(super) fn pop_undo(&mut self) -> Option<PageTransaction> {
        let transaction = self.undo.pop()?;
        self.current_id = self.undo.last().map_or(0, |item| item.id);
        Some(transaction)
    }

    pub(super) fn finish_undo(&mut self, transaction: PageTransaction) {
        self.redo.push(transaction);
    }

    pub(super) fn pop_redo(&mut self) -> Option<PageTransaction> {
        self.redo.pop()
    }

    pub(super) fn finish_redo(&mut self, transaction: PageTransaction) {
        self.current_id = transaction.id;
        self.undo.push(transaction);
    }

    pub(super) fn position(&self) -> u64 {
        self.current_id
    }
}
