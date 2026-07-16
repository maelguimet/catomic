//! Purpose: implement Buffer for editable logical-line file pages.
//! Owns: active-page queries/edits, page navigation, history, and streamed output.
//! Must not: open paths, replace files, parse config, render, or own App policy.
//! Invariants: the synthetic trailing boundary row is hidden; edits remain writable;
//!   global undo/redo returns to the page containing the affected transaction.
//! Phase: 2-by editable paged-file storage.

use std::borrow::Cow;
use std::io::{self, Write};

use crate::buffer::{
    Buffer, Cursor, DescriptorOverlay, DescriptorPosition, DescriptorSource, LineView, PageInfo,
};

use super::PagedFileBuffer;

impl Buffer for PagedFileBuffer {
    fn line_count(&self) -> usize {
        self.visible_line_count()
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        if row >= self.line_count() {
            None
        } else {
            self.active().buffer.line(row)
        }
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let height = height.min(self.line_count().saturating_sub(start));
        self.active().buffer.visible_lines(start, height)
    }

    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView> {
        self.try_visible_lines_window(start, height, start_col, width)
            .unwrap_or_default()
    }

    fn try_visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<Vec<LineView>> {
        let height = height.min(self.line_count().saturating_sub(start));
        self.active()
            .buffer
            .try_visible_lines_window(start, height, start_col, width)
    }

    fn line_char_count(&self, row: usize) -> Option<usize> {
        if row >= self.line_count() {
            None
        } else {
            self.active().buffer.line_char_count(row)
        }
    }

    fn cursor(&self) -> Cursor {
        self.active().buffer.cursor()
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let row = cursor.row.min(self.line_count().saturating_sub(1));
        self.active_mut().buffer.set_cursor(Cursor {
            row,
            col: cursor.col,
        });
    }

    fn page_info(&self) -> Option<PageInfo> {
        let page = self.active();
        Some(PageInfo {
            page_number: page.page_number,
            start_byte: page.start_byte as u64,
            end_byte: page.end_byte as u64,
            total_bytes: self.total_bytes as u64,
            has_previous: page.start_byte > 0,
            has_next: page.next_page_start.is_some(),
        })
    }

    fn next_page(&mut self) -> io::Result<bool> {
        let Some(start) = self.active().next_page_start else {
            return Ok(false);
        };
        let page_number = self.active().page_number + 1;
        self.activate_page(start, page_number)?;
        Ok(true)
    }

    fn previous_page(&mut self) -> io::Result<bool> {
        if self.active().start_byte == 0 {
            return Ok(false);
        }
        let start = self.previous_start()?;
        let page_number = self.active().page_number.saturating_sub(1).max(1);
        self.activate_page(start, page_number)?;
        Ok(true)
    }

    fn descriptor_source(&self) -> io::Result<Option<DescriptorSource>> {
        self.ensure_unchanged()?;
        let mut overlays = Vec::new();
        for page in self.retained.values().chain(std::iter::once(self.active())) {
            if page.buffer.edit_history_position() == 0 {
                continue;
            }
            let mut content = Vec::new();
            page.buffer.write_to(&mut content)?;
            overlays.push(DescriptorOverlay {
                start_byte: page.start_byte as u64,
                end_byte: page.end_byte as u64,
                page_number: page.page_number,
                content,
            });
        }
        overlays.sort_by_key(|overlay| overlay.start_byte);
        Ok(Some(DescriptorSource {
            file: self.file.try_clone()?,
            total_bytes: self.total_bytes as u64,
            page_lines: self.page_lines,
            overlays,
        }))
    }

    fn set_descriptor_position(&mut self, position: DescriptorPosition) -> io::Result<bool> {
        let start = usize::try_from(position.page_start).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "search page offset is too large",
            )
        })?;
        self.activate_page(start, position.page_number)?;
        self.set_cursor(Cursor {
            row: position.row,
            col: position.col,
        });
        Ok(true)
    }

    fn text_range(&self, start: Cursor, end: Cursor) -> io::Result<String> {
        self.active().buffer.text_range(start, end)
    }

    fn replace_range(&mut self, start: Cursor, end: Cursor, text: &str) -> io::Result<bool> {
        let page_start = self.active().start_byte;
        let before = self.active().buffer.edit_history_position();
        let changed = self.active_mut().buffer.replace_range(start, end, text)?;
        let after = self.active().buffer.edit_history_position();
        if changed && after != before {
            self.history.record(page_start);
        }
        Ok(changed)
    }

    fn to_string(&self) -> String {
        self.active().buffer.to_string()
    }

    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.stream_document(out)
    }

    #[cfg(test)]
    fn lines(&self) -> Vec<String> {
        (0..self.line_count())
            .filter_map(|row| self.line(row).map(Cow::into_owned))
            .collect()
    }

    fn insert_char(&mut self, ch: char) {
        self.mutate_active(|buffer| buffer.insert_char(ch));
    }

    fn insert_newline(&mut self) {
        self.mutate_active(Buffer::insert_newline);
    }

    fn delete_back(&mut self) {
        if self.cursor() == (Cursor { row: 0, col: 0 }) && self.active().start_byte > 0 {
            if self.previous_page().unwrap_or(false) {
                let row = self.line_count().saturating_sub(1);
                let col = self.line_char_count(row).unwrap_or(0);
                self.set_cursor(Cursor { row, col });
                if self.hides_boundary_row() {
                    self.mutate_active(Buffer::delete_forward);
                } else {
                    self.mutate_active(Buffer::delete_back);
                }
            }
            return;
        }
        self.mutate_active(Buffer::delete_back);
    }

    fn delete_forward(&mut self) {
        let cursor = self.cursor();
        let last_row = self.line_count().saturating_sub(1);
        let at_page_end =
            cursor.row == last_row && cursor.col == self.line_char_count(last_row).unwrap_or(0);
        if at_page_end && !self.hides_boundary_row() && self.active().next_page_start.is_some() {
            if self.next_page().unwrap_or(false) {
                self.set_cursor(Cursor { row: 0, col: 0 });
                self.mutate_active(Buffer::delete_forward);
            }
            return;
        }
        self.mutate_active(Buffer::delete_forward);
    }

    fn move_left(&mut self) {
        self.active_mut().buffer.move_left();
    }

    fn move_right(&mut self) {
        let cursor = self.cursor();
        let last_row = self.line_count().saturating_sub(1);
        let last_col = self.line_char_count(last_row).unwrap_or(0);
        if cursor.row != last_row || cursor.col != last_col {
            self.active_mut().buffer.move_right();
        }
    }

    fn move_up(&mut self) {
        self.active_mut().buffer.move_up();
    }

    fn move_down(&mut self) {
        if self.cursor().row + 1 < self.line_count() {
            self.active_mut().buffer.move_down();
        }
    }

    fn undo(&mut self) {
        self.undo_active_transaction();
    }

    fn redo(&mut self) {
        self.redo_active_transaction();
    }

    fn edit_history_position(&self) -> u64 {
        self.history.position()
    }
}
