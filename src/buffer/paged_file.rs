//! Purpose: provide editable logical-line pages over one stable file descriptor.
//! Owns: active/retained page lifetime, stable page loading, and cross-page history.
//! Must not: own App policy, path replacement, terminal input/rendering, or external services.
//! Invariants: only pages with edit history or current edits are retained; original
//!   page byte ranges never overlap; descriptor drift fails page loads and whole-file
//!   writes closed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
#[cfg(test)]
use std::path::Path;

use crate::buffer::large_file::page_scan::find_previous_page_start;
use crate::buffer::{Buffer, PieceTable};

mod buffer_impl;
mod history;
mod stream;

use history::{HistoryChange, PageHistory};

pub(crate) struct PagedFileBuffer {
    file: File,
    snapshot: DescriptorSnapshot,
    total_bytes: usize,
    page_lines: usize,
    active: Option<EditablePage>,
    retained: BTreeMap<usize, EditablePage>,
    history: PageHistory,
    #[cfg(test)]
    metadata_check_count: std::cell::Cell<usize>,
}

#[cfg(test)]
pub(crate) struct PagedFilePerfStats {
    pub(crate) active_pages: usize,
    pub(crate) edited_retained_pages: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) retained_page_metadata_bytes: usize,
    pub(crate) descriptor_read_bytes: usize,
    pub(crate) descriptor_metadata_checks: usize,
}

pub(super) struct EditablePage {
    pub(super) buffer: PieceTable,
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) next_page_start: Option<usize>,
    pub(super) page_number: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DescriptorSnapshot {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl DescriptorSnapshot {
    fn capture(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

impl PagedFileBuffer {
    #[cfg(test)]
    pub(crate) fn open(path: impl AsRef<Path>, page_lines: usize) -> io::Result<Self> {
        Self::from_file(File::open(path)?, page_lines)
    }

    pub(crate) fn from_file(file: File, page_lines: usize) -> io::Result<Self> {
        let snapshot = DescriptorSnapshot::capture(&file)?;
        let total_bytes = usize::try_from(snapshot.len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file size exceeds this platform's addressable range",
            )
        })?;
        let first = Self::load_from_descriptor(&file, 0, 1, page_lines)?;
        if DescriptorSnapshot::capture(&file)? != snapshot {
            return Err(changed_descriptor_error());
        }
        Ok(Self {
            file,
            snapshot,
            total_bytes,
            page_lines,
            active: Some(first),
            retained: BTreeMap::new(),
            history: PageHistory::new(),
            #[cfg(test)]
            metadata_check_count: std::cell::Cell::new(0),
        })
    }

    fn load_from_descriptor(
        file: &File,
        start_byte: usize,
        page_number: usize,
        page_lines: usize,
    ) -> io::Result<EditablePage> {
        let mut page = PieceTable::from_file_page(file.try_clone()?, start_byte, page_lines)?;
        page.buffer.use_external_history_retention();
        Ok(EditablePage {
            buffer: page.buffer,
            start_byte: page.start_byte,
            end_byte: page.end_byte,
            next_page_start: page.next_page_start,
            page_number,
        })
    }

    pub(super) fn active(&self) -> &EditablePage {
        self.active.as_ref().expect("paged buffer has active page")
    }

    pub(super) fn active_mut(&mut self) -> &mut EditablePage {
        self.active.as_mut().expect("paged buffer has active page")
    }

    pub(super) fn visible_line_count(&self) -> usize {
        let page = self.active();
        let count = page.buffer.line_count();
        if self.hides_boundary_row() {
            count - 1
        } else {
            count
        }
    }

    pub(super) fn hides_boundary_row(&self) -> bool {
        let page = self.active();
        let count = page.buffer.line_count();
        page.next_page_start.is_some()
            && count > 1
            && page.buffer.line_char_count(count - 1) == Some(0)
    }

    pub(super) fn ensure_unchanged(&self) -> io::Result<()> {
        #[cfg(test)]
        self.metadata_check_count
            .set(self.metadata_check_count.get() + 1);
        if DescriptorSnapshot::capture(&self.file)? == self.snapshot {
            Ok(())
        } else {
            Err(changed_descriptor_error())
        }
    }

    fn park_active(&mut self) {
        let page = self.active.take().expect("paged buffer has active page");
        if page.buffer.needs_page_retention() {
            self.retained.insert(page.start_byte, page);
        }
    }

    pub(super) fn activate_page(
        &mut self,
        start_byte: usize,
        page_number: usize,
    ) -> io::Result<()> {
        if self.active().start_byte == start_byte {
            self.active_mut().page_number = page_number;
            return Ok(());
        }
        let page = if let Some(mut retained) = self.retained.remove(&start_byte) {
            retained.page_number = page_number;
            retained
        } else {
            self.ensure_unchanged()?;
            let page =
                Self::load_from_descriptor(&self.file, start_byte, page_number, self.page_lines)?;
            self.ensure_unchanged()?;
            page
        };
        self.active_mut().buffer.finish_undo_group();
        self.park_active();
        self.active = Some(page);
        Ok(())
    }

    fn activate_retained(&mut self, start_byte: usize) -> bool {
        if self.active().start_byte == start_byte {
            return true;
        }
        let Some(page) = self.retained.remove(&start_byte) else {
            return false;
        };
        self.active_mut().buffer.finish_undo_group();
        self.park_active();
        self.active = Some(page);
        true
    }

    pub(super) fn previous_start(&self) -> io::Result<usize> {
        self.ensure_unchanged()?;
        let start =
            find_previous_page_start(&self.file, self.active().start_byte, self.page_lines)?;
        self.ensure_unchanged()?;
        Ok(start)
    }

    pub(super) fn mutate_active(&mut self, edit: impl FnOnce(&mut PieceTable)) {
        let start = self.active().start_byte;
        let before_transactions = self.active().buffer.undo_transaction_count();
        let before_revision = self.active().buffer.content_revision();
        edit(&mut self.active_mut().buffer);
        let after_transactions = self.active().buffer.undo_transaction_count();
        let after_revision = self.active().buffer.content_revision();
        if after_revision != before_revision {
            self.record_page_transaction(start, after_transactions == before_transactions);
            self.history.note_content_change();
        }
    }

    fn record_page_transaction(&mut self, page_start: usize, extends_current: bool) {
        let retained_bytes = self
            .active()
            .buffer
            .latest_undo_retained_bytes()
            .expect("changed page has a local undo transaction");
        let change = if extends_current {
            self.history.extend_current(page_start, retained_bytes)
        } else {
            self.history.record(page_start, retained_bytes)
        };
        self.apply_history_change(change);
    }

    fn apply_history_change(&mut self, change: HistoryChange) {
        let invalidated_pages = change
            .invalidated_redo
            .iter()
            .map(|transaction| transaction.page_start)
            .collect::<BTreeSet<_>>();
        for start in invalidated_pages {
            self.page_buffer_mut(start).clear_redo_history();
        }

        let mut pruned_per_page = BTreeMap::new();
        for transaction in change.pruned_undo {
            *pruned_per_page
                .entry(transaction.page_start)
                .or_insert(0usize) += 1;
        }
        for (start, count) in pruned_per_page {
            self.page_buffer_mut(start)
                .discard_oldest_undo_transactions(count);
        }
        self.retained
            .retain(|_, page| page.buffer.needs_page_retention());
    }

    fn page_buffer_mut(&mut self, start_byte: usize) -> &mut PieceTable {
        if self.active().start_byte == start_byte {
            return &mut self.active_mut().buffer;
        }
        &mut self
            .retained
            .get_mut(&start_byte)
            .expect("globally retained history page must remain resident")
            .buffer
    }

    pub(super) fn undo_active_transaction(&mut self) {
        let Some(transaction) = self.history.pop_undo() else {
            return;
        };
        assert!(
            self.activate_retained(transaction.page_start),
            "edited page must remain retained for undo"
        );
        self.active_mut().buffer.undo();
        self.history.finish_undo(transaction);
        self.history.note_content_change();
    }

    pub(super) fn redo_active_transaction(&mut self) {
        let Some(transaction) = self.history.pop_redo() else {
            return;
        };
        assert!(
            self.activate_retained(transaction.page_start),
            "edited page must remain retained for redo"
        );
        self.active_mut().buffer.redo();
        self.history.finish_redo(transaction);
        self.history.note_content_change();
    }

    #[cfg(test)]
    pub(crate) fn perf_stats(&self) -> PagedFilePerfStats {
        let pages = self.retained.values().chain(std::iter::once(self.active()));
        let mut retained_bytes = self.history.allocated_bytes();
        let mut retained_page_metadata_bytes = 0;
        let mut descriptor_read_bytes = 0;
        let mut descriptor_metadata_checks = self.metadata_check_count.get();
        for page in pages {
            let stats = page.buffer.perf_stats();
            retained_bytes += stats.retained_bytes;
            retained_page_metadata_bytes += stats.retained_metadata_bytes;
            descriptor_read_bytes += stats.descriptor_read_bytes;
            descriptor_metadata_checks += stats.descriptor_metadata_checks;
        }
        PagedFilePerfStats {
            active_pages: if self.active.is_some() { 1 } else { 0 },
            edited_retained_pages: self.retained.len(),
            retained_bytes,
            retained_page_metadata_bytes,
            descriptor_read_bytes,
            descriptor_metadata_checks,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_history_retention_for_test(
        &mut self,
        max_transactions: usize,
        max_bytes: usize,
    ) {
        let change = self
            .history
            .set_retention_policy(max_transactions, max_bytes);
        self.apply_history_change(change);
    }

    #[cfg(test)]
    pub(crate) fn retained_page_count_for_test(&self) -> usize {
        self.retained.len()
    }

    #[cfg(test)]
    pub(crate) fn history_retention_metrics_for_test(&self) -> (usize, usize) {
        self.history.retention_metrics()
    }
}

fn changed_descriptor_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "paged file changed while open")
}
