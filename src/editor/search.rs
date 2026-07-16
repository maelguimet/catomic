//! Purpose: find scalar-positioned matches in ordinary and descriptor-backed buffers.
//! Owns: local direction/wrap rules, explicit descriptor worker lifetime,
//!   cancellation, chunked UTF-8 validation, and cross-chunk position tracking.
//! Must not: render, mutate App/Buffer state, reopen paths, index projects, or network.
//! Invariants: descriptor bytes are processed once with bounded memory; matches
//!   can cross read boundaries; result positions use configured logical-line pages.
//! Phase: 3-a incremental search foundation over the Phase 2 descriptor scanner.

use std::collections::VecDeque;
use std::io;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crate::buffer::{Buffer, Cursor, DescriptorPosition, DescriptorSource};
const SEARCH_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SearchMatch {
    pub(crate) start: Cursor,
    pub(crate) end_col: usize,
}

pub(crate) enum SearchResult {
    Found(DescriptorPosition),
    NotFound,
    Error(String),
}

pub(crate) struct SearchTask {
    receiver: mpsc::Receiver<SearchResult>,
    cancel: Arc<AtomicBool>,
}

impl SearchTask {
    pub(crate) fn try_result(&self) -> Option<SearchResult> {
        self.receiver.try_recv().ok()
    }

    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for SearchTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn start_descriptor_search(source: DescriptorSource, query: String) -> SearchTask {
    start_descriptor_search_with(source, query, None, SearchDirection::Forward)
}

pub(crate) fn start_descriptor_search_from(
    source: DescriptorSource,
    query: String,
    anchor: DescriptorPosition,
    direction: SearchDirection,
) -> SearchTask {
    start_descriptor_search_with(source, query, Some(anchor), direction)
}

fn start_descriptor_search_with(
    source: DescriptorSource,
    query: String,
    anchor: Option<DescriptorPosition>,
    direction: SearchDirection,
) -> SearchTask {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let result = scan_descriptor_with(source, &query, &worker_cancel, anchor, direction)
            .unwrap_or_else(|error| SearchResult::Error(error.to_string()));
        if !worker_cancel.load(Ordering::Acquire) {
            let _ = sender.send(result);
        }
    });
    SearchTask { receiver, cancel }
}

pub(crate) fn find_match(
    buffer: &dyn Buffer,
    query: &str,
    origin: Cursor,
    direction: SearchDirection,
    include_origin: bool,
) -> Option<SearchMatch> {
    if query.is_empty() || query.contains('\n') {
        return None;
    }
    let mut first = None;
    let mut last = None;
    let mut before_origin = None;
    for row in 0..buffer.line_count() {
        let line = buffer.line(row)?;
        for (byte_col, _) in line.match_indices(query) {
            let start = Cursor {
                row,
                col: line[..byte_col].chars().count(),
            };
            let found = SearchMatch {
                start,
                end_col: start.col + query.chars().count(),
            };
            first.get_or_insert(found);
            last = Some(found);
            let ordering = compare_cursor(start, origin);
            match direction {
                SearchDirection::Forward
                    if ordering.is_gt() || (include_origin && ordering.is_eq()) =>
                {
                    return Some(found);
                }
                SearchDirection::Backward
                    if ordering.is_lt() || (include_origin && ordering.is_eq()) =>
                {
                    before_origin = Some(found);
                }
                _ => {}
            }
        }
    }
    match direction {
        SearchDirection::Forward => first,
        SearchDirection::Backward => before_origin.or(last),
    }
}

fn compare_cursor(left: Cursor, right: Cursor) -> std::cmp::Ordering {
    (left.row, left.col).cmp(&(right.row, right.col))
}

#[cfg(test)]
fn scan_descriptor(
    source: DescriptorSource,
    query: &str,
    cancel: &AtomicBool,
) -> io::Result<SearchResult> {
    scan_descriptor_with(source, query, cancel, None, SearchDirection::Forward)
}

#[cfg(test)]
fn scan_descriptor_from(
    source: DescriptorSource,
    query: &str,
    cancel: &AtomicBool,
    anchor: DescriptorPosition,
    direction: SearchDirection,
) -> io::Result<SearchResult> {
    scan_descriptor_with(source, query, cancel, Some(anchor), direction)
}

fn scan_descriptor_with(
    source: DescriptorSource,
    query: &str,
    cancel: &AtomicBool,
    anchor: Option<DescriptorPosition>,
    direction: SearchDirection,
) -> io::Result<SearchResult> {
    if query.is_empty() || query.contains('\n') {
        return Ok(SearchResult::NotFound);
    }
    let initial_meta = source.file.metadata()?;
    if initial_meta.len() != source.total_bytes {
        return Err(changed_file_error());
    }
    let initial_modified = initial_meta.modified().ok();
    let mut scanner = Scanner::new(query, source.page_lines, anchor, direction);
    let mut chunk = vec![0u8; SEARCH_CHUNK_BYTES];
    let mut carry = Vec::new();
    let mut offset = 0u64;
    let mut overlay_index = 0usize;
    while offset < source.total_bytes {
        if cancel.load(Ordering::Acquire) {
            return Ok(SearchResult::NotFound);
        }
        if let Some(overlay) = source.overlays.get(overlay_index) {
            validate_overlay(overlay, offset, source.total_bytes)?;
            if overlay.start_byte == offset {
                if !carry.is_empty() {
                    return Err(changed_file_error());
                }
                let text = std::str::from_utf8(&overlay.content)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                scanner.begin_page(overlay.start_byte, overlay.page_number);
                if let Some(position) = scanner.scan_fixed_page_text(text) {
                    ensure_unchanged(&source, initial_modified)?;
                    return Ok(SearchResult::Found(position));
                }
                offset = overlay.end_byte;
                scanner.begin_page(offset, overlay.page_number + 1);
                overlay_index += 1;
                continue;
            }
        }
        let read_limit = source
            .overlays
            .get(overlay_index)
            .map_or(chunk.len(), |overlay| {
                usize::try_from(overlay.start_byte - offset)
                    .unwrap_or(chunk.len())
                    .min(chunk.len())
            });
        let read = source.file.read_at(&mut chunk[..read_limit], offset)?;
        if read == 0 {
            return Err(changed_file_error());
        }
        let carry_len = carry.len();
        let text_start = offset.saturating_sub(carry_len as u64);
        let mut combined;
        let bytes = if carry.is_empty() {
            &chunk[..read]
        } else {
            combined = Vec::with_capacity(carry_len + read);
            combined.extend_from_slice(&carry);
            combined.extend_from_slice(&chunk[..read]);
            carry.clear();
            &combined
        };
        let valid_end = valid_utf8_end(bytes)?;
        let text = std::str::from_utf8(&bytes[..valid_end])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some(position) = scanner.scan_text(text, text_start) {
            ensure_unchanged(&source, initial_modified)?;
            return Ok(SearchResult::Found(position));
        }
        carry.extend_from_slice(&bytes[valid_end..]);
        offset += read as u64;
    }
    if !carry.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete utf-8 sequence at end of file",
        ));
    }
    ensure_unchanged(&source, initial_modified)?;
    Ok(scanner
        .finish()
        .map_or(SearchResult::NotFound, SearchResult::Found))
}

fn validate_overlay(
    overlay: &crate::buffer::DescriptorOverlay,
    offset: u64,
    total_bytes: u64,
) -> io::Result<()> {
    if overlay.start_byte < offset
        || overlay.start_byte >= overlay.end_byte
        || overlay.end_byte > total_bytes
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid edited page range during search",
        ))
    } else {
        Ok(())
    }
}

fn ensure_unchanged(
    source: &DescriptorSource,
    initial_modified: Option<std::time::SystemTime>,
) -> io::Result<()> {
    let meta = source.file.metadata()?;
    if meta.len() == source.total_bytes && meta.modified().ok() == initial_modified {
        Ok(())
    } else {
        Err(changed_file_error())
    }
}

fn changed_file_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "large file changed during search",
    )
}

fn valid_utf8_end(bytes: &[u8]) -> io::Result<usize> {
    match std::str::from_utf8(bytes) {
        Ok(_) => Ok(bytes.len()),
        Err(error) if error.error_len().is_none() => Ok(error.valid_up_to()),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}

struct Scanner {
    query: Vec<u8>,
    prefix: Vec<usize>,
    matched: usize,
    recent_positions: VecDeque<DescriptorPosition>,
    page_lines: usize,
    page_start: u64,
    page_number: usize,
    row: usize,
    col: usize,
    anchor: Option<DescriptorPosition>,
    direction: SearchDirection,
    first_match: Option<DescriptorPosition>,
    last_match: Option<DescriptorPosition>,
    before_anchor: Option<DescriptorPosition>,
}

impl Scanner {
    fn new(
        query: &str,
        page_lines: usize,
        anchor: Option<DescriptorPosition>,
        direction: SearchDirection,
    ) -> Self {
        Self {
            query: query.as_bytes().to_vec(),
            prefix: prefix_table(query.as_bytes()),
            matched: 0,
            recent_positions: VecDeque::with_capacity(query.len()),
            page_lines,
            page_start: 0,
            page_number: 1,
            row: 0,
            col: 0,
            anchor,
            direction,
            first_match: None,
            last_match: None,
            before_anchor: None,
        }
    }

    fn scan_text(&mut self, text: &str, text_start: u64) -> Option<DescriptorPosition> {
        self.scan_text_with_page_boundaries(text, text_start, true)
    }

    fn begin_page(&mut self, page_start: u64, page_number: usize) {
        self.page_start = page_start;
        self.page_number = page_number;
        self.row = 0;
        self.col = 0;
    }

    fn scan_fixed_page_text(&mut self, text: &str) -> Option<DescriptorPosition> {
        self.scan_text_with_page_boundaries(text, self.page_start, false)
    }

    fn scan_text_with_page_boundaries(
        &mut self,
        text: &str,
        text_start: u64,
        advance_pages: bool,
    ) -> Option<DescriptorPosition> {
        for (byte_index, ch) in text.char_indices() {
            let mut encoded = [0; 4];
            let position = self.current_position();
            for byte in ch.encode_utf8(&mut encoded).as_bytes() {
                if let Some(found) = self.feed(*byte, position) {
                    if let Some(selected) = self.consider(found) {
                        return Some(selected);
                    }
                }
            }
            if ch == '\n' {
                self.row += 1;
                self.col = 0;
                if advance_pages && self.row == self.page_lines {
                    self.page_start = text_start + byte_index as u64 + 1;
                    self.page_number += 1;
                    self.row = 0;
                }
            } else {
                self.col += 1;
            }
        }
        None
    }

    fn current_position(&self) -> DescriptorPosition {
        DescriptorPosition {
            page_start: self.page_start,
            page_number: self.page_number,
            row: self.row,
            col: self.col,
        }
    }

    fn feed(&mut self, byte: u8, position: DescriptorPosition) -> Option<DescriptorPosition> {
        self.recent_positions.push_back(position);
        if self.recent_positions.len() > self.query.len() {
            self.recent_positions.pop_front();
        }
        while self.matched > 0 && self.query[self.matched] != byte {
            self.matched = self.prefix[self.matched - 1];
        }
        if self.query[self.matched] == byte {
            self.matched += 1;
        }
        if self.matched == self.query.len() {
            let position = *self
                .recent_positions
                .front()
                .expect("a complete match has a start position");
            self.matched = self.prefix[self.matched - 1];
            Some(position)
        } else {
            None
        }
    }

    fn consider(&mut self, found: DescriptorPosition) -> Option<DescriptorPosition> {
        self.first_match.get_or_insert(found);
        self.last_match = Some(found);
        let Some(anchor) = self.anchor else {
            return Some(found);
        };
        match self.direction {
            SearchDirection::Forward if compare_descriptor_position(found, anchor).is_gt() => {
                Some(found)
            }
            SearchDirection::Backward if compare_descriptor_position(found, anchor).is_lt() => {
                self.before_anchor = Some(found);
                None
            }
            _ => None,
        }
    }

    fn finish(&self) -> Option<DescriptorPosition> {
        match (self.anchor, self.direction) {
            (None, _) | (Some(_), SearchDirection::Forward) => self.first_match,
            (Some(_), SearchDirection::Backward) => self.before_anchor.or(self.last_match),
        }
    }
}

fn compare_descriptor_position(
    left: DescriptorPosition,
    right: DescriptorPosition,
) -> std::cmp::Ordering {
    (left.page_start, left.row, left.col).cmp(&(right.page_start, right.row, right.col))
}

fn prefix_table(query: &[u8]) -> Vec<usize> {
    let mut prefix = vec![0; query.len()];
    let mut matched = 0usize;
    for index in 1..query.len() {
        while matched > 0 && query[index] != query[matched] {
            matched = prefix[matched - 1];
        }
        if query[index] == query[matched] {
            matched += 1;
            prefix[index] = matched;
        }
    }
    prefix
}

#[cfg(test)]
mod tests;
