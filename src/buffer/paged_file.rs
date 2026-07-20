//! Purpose: provide editable logical-line pages over one stable file descriptor.
//! Owns: active/retained page lifetime, stable page loading, and cross-page history.
//! Must not: own App policy, path replacement, terminal input/rendering, Project, or LLM.
//! Invariants: only pages with edit history are retained; original page byte ranges
//!   never overlap; descriptor drift fails page loads and whole-file writes closed.
//! Phase: 2-by editable paged-file storage.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
#[cfg(test)]
use std::path::Path;

use crate::buffer::large_file::page_scan::find_previous_page_start;
use crate::buffer::{Buffer, PieceTable};

mod buffer_impl;
mod history;
mod stream;

use history::PageHistory;

pub(crate) struct PagedFileBuffer {
    file: File,
    snapshot: DescriptorSnapshot,
    total_bytes: usize,
    page_lines: usize,
    active: Option<EditablePage>,
    retained: BTreeMap<usize, EditablePage>,
    history: PageHistory,
    #[cfg(test)]
    fail_next_page_after: Option<(usize, io::ErrorKind)>,
    #[cfg(test)]
    fail_previous_page_once: Option<io::ErrorKind>,
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
            fail_next_page_after: None,
            #[cfg(test)]
            fail_previous_page_once: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn fail_next_page_after(&mut self, successful_calls: usize, kind: io::ErrorKind) {
        self.fail_next_page_after = Some((successful_calls, kind));
    }

    #[cfg(test)]
    pub(crate) fn fail_previous_page_once(&mut self, kind: io::ErrorKind) {
        self.fail_previous_page_once = Some(kind);
    }

    fn load_from_descriptor(
        file: &File,
        start_byte: usize,
        page_number: usize,
        page_lines: usize,
    ) -> io::Result<EditablePage> {
        let page = PieceTable::from_file_page(file.try_clone()?, start_byte, page_lines)?;
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
        if DescriptorSnapshot::capture(&self.file)? == self.snapshot {
            Ok(())
        } else {
            Err(changed_descriptor_error())
        }
    }

    fn park_active(&mut self) {
        let page = self.active.take().expect("paged buffer has active page");
        if page.buffer.has_edit_history() {
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
        let before = self.active().buffer.edit_history_position();
        edit(&mut self.active_mut().buffer);
        let after = self.active().buffer.edit_history_position();
        if after != before {
            self.history.record(start);
        }
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
    }
}

fn changed_descriptor_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "paged file changed while open")
}
