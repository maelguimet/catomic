//! Purpose: track bounded cross-page edit order without snapshots or content copies.
//! Owns: global undo/redo ordering, exact history-position tokens, content revisions,
//! and retention decisions synchronized into page-local PieceTables.
//! Must not: edit pages, read descriptors, render, save, or know App state.
//! Invariants: transaction ids are stable across undo/redo; extending a grouped
//! edit refreshes its id without adding a transaction; pruning never removes the
//! newest transaction even when it alone exceeds a budget.

use std::collections::VecDeque;
use std::mem::size_of;

use crate::buffer::undo::UndoRetentionPolicy;

#[derive(Clone, Copy)]
pub(super) struct PageTransaction {
    pub(super) page_start: usize,
    retained_bytes: usize,
    id: u64,
}

pub(super) struct HistoryChange {
    pub(super) invalidated_redo: Vec<PageTransaction>,
    pub(super) pruned_undo: Vec<PageTransaction>,
}

impl HistoryChange {
    fn unchanged() -> Self {
        Self {
            invalidated_redo: Vec::new(),
            pruned_undo: Vec::new(),
        }
    }
}

pub(super) struct PageHistory {
    undo: VecDeque<PageTransaction>,
    redo: VecDeque<PageTransaction>,
    base_id: u64,
    next_id: u64,
    current_id: u64,
    revision: u64,
    retained_estimate_bytes: usize,
    policy: UndoRetentionPolicy,
}

impl PageHistory {
    pub(super) fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            base_id: 0,
            next_id: 1,
            current_id: 0,
            revision: 0,
            retained_estimate_bytes: 0,
            policy: UndoRetentionPolicy::default(),
        }
    }

    pub(super) fn record(&mut self, page_start: usize, retained_bytes: usize) -> HistoryChange {
        let mut change = HistoryChange {
            invalidated_redo: self.redo.drain(..).collect(),
            pruned_undo: Vec::new(),
        };
        for transaction in &change.invalidated_redo {
            self.retained_estimate_bytes = self
                .retained_estimate_bytes
                .saturating_sub(transaction.retained_bytes);
        }

        let retained_bytes = retained_bytes.saturating_add(size_of::<PageTransaction>());
        let transaction = PageTransaction {
            page_start,
            retained_bytes,
            id: self.next_id,
        };
        self.next_id += 1;
        self.current_id = transaction.id;
        self.retained_estimate_bytes = self.retained_estimate_bytes.saturating_add(retained_bytes);
        self.undo.push_back(transaction);
        change.pruned_undo = self.prune_undo();
        change
    }

    pub(super) fn extend_current(
        &mut self,
        page_start: usize,
        retained_bytes: usize,
    ) -> HistoryChange {
        let Some(transaction) = self.undo.back_mut() else {
            return self.record(page_start, retained_bytes);
        };
        assert_eq!(
            transaction.page_start, page_start,
            "only the newest global page transaction may extend"
        );
        let retained_bytes = retained_bytes.saturating_add(size_of::<PageTransaction>());
        self.retained_estimate_bytes = self
            .retained_estimate_bytes
            .saturating_sub(transaction.retained_bytes)
            .saturating_add(retained_bytes);
        transaction.retained_bytes = retained_bytes;
        transaction.id = self.next_id;
        self.next_id += 1;
        self.current_id = transaction.id;

        let mut change = HistoryChange::unchanged();
        change.pruned_undo = self.prune_undo();
        change
    }

    fn prune_undo(&mut self) -> Vec<PageTransaction> {
        let mut pruned = Vec::new();
        while self.undo.len() > 1
            && (self.undo.len() > self.policy.max_transactions
                || self.retained_estimate_bytes > self.policy.max_bytes)
        {
            let transaction = self
                .undo
                .pop_front()
                .expect("multiple undo transactions have an oldest transaction");
            self.base_id = transaction.id;
            self.retained_estimate_bytes = self
                .retained_estimate_bytes
                .saturating_sub(transaction.retained_bytes);
            pruned.push(transaction);
        }
        pruned
    }

    pub(super) fn pop_undo(&mut self) -> Option<PageTransaction> {
        let transaction = self.undo.pop_back()?;
        self.current_id = self.undo.back().map_or(self.base_id, |item| item.id);
        Some(transaction)
    }

    pub(super) fn finish_undo(&mut self, transaction: PageTransaction) {
        self.redo.push_back(transaction);
    }

    pub(super) fn pop_redo(&mut self) -> Option<PageTransaction> {
        self.redo.pop_back()
    }

    pub(super) fn finish_redo(&mut self, transaction: PageTransaction) {
        self.current_id = transaction.id;
        self.undo.push_back(transaction);
    }

    pub(super) fn position(&self) -> u64 {
        self.current_id
    }

    pub(super) fn is_position_retained(&self, position: u64) -> bool {
        position == self.base_id
            || self
                .undo
                .iter()
                .any(|transaction| transaction.id == position)
            || self
                .redo
                .iter()
                .any(|transaction| transaction.id == position)
    }

    pub(super) fn note_content_change(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn content_revision(&self) -> u64 {
        self.revision
    }

    #[cfg(test)]
    pub(super) fn allocated_bytes(&self) -> usize {
        (self.undo.capacity() + self.redo.capacity()) * size_of::<PageTransaction>()
    }

    #[cfg(test)]
    pub(super) fn retention_metrics(&self) -> (usize, usize) {
        (self.undo.len(), self.retained_estimate_bytes)
    }

    #[cfg(test)]
    pub(super) fn set_retention_policy(
        &mut self,
        max_transactions: usize,
        max_bytes: usize,
    ) -> HistoryChange {
        self.policy = UndoRetentionPolicy {
            max_transactions,
            max_bytes,
        };
        let mut change = HistoryChange::unchanged();
        change.pruned_undo = self.prune_undo();
        change
    }
}
