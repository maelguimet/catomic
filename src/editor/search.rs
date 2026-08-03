//! Purpose: find scalar-positioned matches in ordinary and descriptor-backed buffers.
//! Owns: local direction/wrap rules, explicit descriptor worker lifetime,
//!   cancellation, chunked UTF-8 validation, and cross-chunk position tracking.
//! Must not: render, mutate App/Buffer state, reopen paths, index projects, or network.
//! Invariants: descriptor bytes are processed once with bounded memory; matches
//!   can cross read boundaries; result positions use configured logical-line pages.

use std::io;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use memchr::memchr_iter;

use crate::buffer::{Buffer, Cursor, DescriptorPosition, DescriptorSource};

mod literal;

use literal::LiteralByteMatcher;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorSearchMatch {
    pub(crate) position: DescriptorPosition,
    pub(crate) byte_offset: usize,
}

pub(crate) enum SearchResult {
    Found(DescriptorSearchMatch),
    LocalFound(SearchMatch),
    NotFound,
    Error(String),
}

/// Incremental literal scanner for an in-memory buffer. Literal matching stays
/// byte-oriented; the buffer owner converts only the origin and selected byte
/// offsets through its focused search-coordinate seam.
pub(crate) struct LocalSearchTask {
    matcher: LiteralByteMatcher,
    query_scalar_len: usize,
    origin: Cursor,
    origin_byte: Option<usize>,
    direction: SearchDirection,
    include_origin: bool,
    first: Option<usize>,
    last: Option<usize>,
    before_origin: Option<usize>,
    cancelled: bool,
    invalid_query: bool,
}

impl LocalSearchTask {
    pub(crate) fn new(
        query: &str,
        origin: Cursor,
        direction: SearchDirection,
        include_origin: bool,
    ) -> Self {
        Self {
            matcher: LiteralByteMatcher::new(query.as_bytes()),
            query_scalar_len: query.chars().count(),
            origin,
            origin_byte: None,
            direction,
            include_origin,
            first: None,
            last: None,
            before_origin: None,
            cancelled: false,
            invalid_query: query.is_empty() || query.contains('\n'),
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub(crate) fn poll(&mut self, buffer: &dyn Buffer, budget: usize) -> Option<SearchResult> {
        if self.cancelled || self.invalid_query {
            return Some(SearchResult::NotFound);
        }
        let Some(source) = buffer.piece_table_search() else {
            return Some(SearchResult::Error(
                "buffer does not expose incremental PieceTable search".to_owned(),
            ));
        };
        if self.origin_byte.is_none() {
            match source.byte_offset_for_cursor(self.origin) {
                Ok(origin) => self.origin_byte = Some(origin),
                Err(error) => return Some(SearchResult::Error(error.to_string())),
            }
        }
        let mut remaining = budget;
        while remaining > 0 {
            let Some(segment) = source.text_segment(self.matcher.processed_bytes(), remaining)
            else {
                return Some(self.finish(source));
            };
            if segment.is_empty() {
                return Some(self.finish(source));
            }
            let length = segment.len();
            let stop_at_or_after = match self.direction {
                SearchDirection::Forward if self.include_origin => self.origin_byte,
                SearchDirection::Forward => {
                    self.origin_byte.and_then(|origin| origin.checked_add(1))
                }
                SearchDirection::Backward => None,
            };
            self.matcher
                .find_segment_matches(segment.as_bytes(), stop_at_or_after);
            let origin = self.origin_byte.unwrap_or(0);
            let selected = {
                let first = &mut self.first;
                let last = &mut self.last;
                let before_origin = &mut self.before_origin;
                let mut selected = None;
                for &offset in self.matcher.candidates() {
                    if let Some(found) = consider_local_candidate(
                        offset,
                        origin,
                        self.direction,
                        self.include_origin,
                        first,
                        last,
                        before_origin,
                    ) {
                        selected = Some(found);
                        break;
                    }
                }
                selected
            };
            if let Some(selected) = selected {
                return Some(self.match_at(source, selected));
            }
            let retained_start = self.matcher.retained_start(segment.as_bytes());
            self.matcher
                .commit_segment(segment.as_bytes(), retained_start);
            remaining = remaining.saturating_sub(length);
        }
        None
    }

    fn finish(&self, source: crate::buffer::PieceTableSearch<'_>) -> SearchResult {
        let selected = match self.direction {
            SearchDirection::Forward => self.first,
            SearchDirection::Backward => self.before_origin.or(self.last),
        };
        selected.map_or(SearchResult::NotFound, |offset| {
            self.match_at(source, offset)
        })
    }

    fn match_at(&self, source: crate::buffer::PieceTableSearch<'_>, offset: usize) -> SearchResult {
        match source.cursor_for_byte_offset(offset) {
            Ok(start) => SearchResult::LocalFound(SearchMatch {
                start,
                end_col: start.col + self.query_scalar_len,
            }),
            Err(error) => SearchResult::Error(error.to_string()),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_overlap_bytes(&self) -> usize {
        self.matcher.retained_bytes()
    }
}

#[allow(clippy::too_many_arguments)]
fn consider_local_candidate(
    offset: usize,
    origin: usize,
    direction: SearchDirection,
    include_origin: bool,
    first: &mut Option<usize>,
    last: &mut Option<usize>,
    before_origin: &mut Option<usize>,
) -> Option<usize> {
    first.get_or_insert(offset);
    *last = Some(offset);
    match direction {
        SearchDirection::Forward if offset > origin || (include_origin && offset == origin) => {
            Some(offset)
        }
        SearchDirection::Backward if offset < origin || (include_origin && offset == origin) => {
            *before_origin = Some(offset);
            None
        }
        _ => None,
    }
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
    anchor: DescriptorSearchMatch,
    direction: SearchDirection,
) -> SearchTask {
    start_descriptor_search_with(source, query, Some(anchor), direction)
}

fn start_descriptor_search_with(
    source: DescriptorSource,
    query: String,
    anchor: Option<DescriptorSearchMatch>,
    direction: SearchDirection,
) -> SearchTask {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let result = scan_descriptor_with(&source, &query, &worker_cancel, anchor, direction)
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
    let query_scalar_len = query.chars().count();
    let finder = memchr::memmem::Finder::new(query.as_bytes());
    let mut first = None;
    let mut last = None;
    let mut before_origin = None;
    for row in 0..buffer.line_count() {
        let line = buffer.line(row)?;
        let mut search_start = 0usize;
        while search_start.saturating_add(query.len()) <= line.len() {
            let Some(relative) = finder.find(&line.as_bytes()[search_start..]) else {
                break;
            };
            let byte_col = search_start + relative;
            let start = Cursor {
                row,
                col: line[..byte_col].chars().count(),
            };
            let found = SearchMatch {
                start,
                end_col: start.col + query_scalar_len,
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
            search_start = byte_col + 1;
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
    scan_descriptor_with(&source, query, cancel, None, SearchDirection::Forward)
}

#[cfg(test)]
fn scan_descriptor_from(
    source: DescriptorSource,
    query: &str,
    cancel: &AtomicBool,
    anchor: DescriptorSearchMatch,
    direction: SearchDirection,
) -> io::Result<SearchResult> {
    scan_descriptor_with(&source, query, cancel, Some(anchor), direction)
}

#[cfg(test)]
pub(crate) fn scan_descriptor_for_perf(
    source: &DescriptorSource,
    query: &str,
    anchor: Option<DescriptorSearchMatch>,
    direction: SearchDirection,
) -> io::Result<SearchResult> {
    scan_descriptor_with(source, query, &AtomicBool::new(false), anchor, direction)
}

fn scan_descriptor_with(
    source: &DescriptorSource,
    query: &str,
    cancel: &AtomicBool,
    anchor: Option<DescriptorSearchMatch>,
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
                let mut text_offset = 0usize;
                while text_offset < text.len() {
                    if cancel.load(Ordering::Acquire) {
                        return Ok(SearchResult::NotFound);
                    }
                    let mut text_end = text_offset
                        .saturating_add(SEARCH_CHUNK_BYTES)
                        .min(text.len());
                    while text_end > text_offset && !text.is_char_boundary(text_end) {
                        text_end -= 1;
                    }
                    if let Some(position) =
                        scanner.scan_fixed_page_text(&text[text_offset..text_end])?
                    {
                        ensure_unchanged(source, initial_modified)?;
                        return Ok(SearchResult::Found(position));
                    }
                    text_offset = text_end;
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
        if let Some(position) = scanner.scan_text(text, text_start)? {
            ensure_unchanged(source, initial_modified)?;
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
    ensure_unchanged(source, initial_modified)?;
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
    matcher: LiteralByteMatcher,
    page_lines: usize,
    position: DescriptorPosition,
    coordinate_spans: Vec<DescriptorCoordinateSpan>,
    selection: DescriptorSelection,
}

#[derive(Clone, Copy)]
struct DescriptorCoordinateSpan {
    stream_start: usize,
    stream_end: usize,
    position: DescriptorPosition,
    text_start: u64,
    advance_pages: bool,
}

struct DescriptorSelection {
    anchor: Option<usize>,
    direction: SearchDirection,
    first_match: Option<DescriptorSearchMatch>,
    last_match: Option<DescriptorSearchMatch>,
    before_anchor: Option<DescriptorSearchMatch>,
}

impl Scanner {
    fn new(
        query: &str,
        page_lines: usize,
        anchor: Option<DescriptorSearchMatch>,
        direction: SearchDirection,
    ) -> Self {
        Self {
            matcher: LiteralByteMatcher::new(query.as_bytes()),
            page_lines,
            position: DescriptorPosition {
                page_start: 0,
                page_number: 1,
                row: 0,
                col: 0,
            },
            coordinate_spans: Vec::new(),
            selection: DescriptorSelection {
                anchor: anchor.map(|found| found.byte_offset),
                direction,
                first_match: None,
                last_match: None,
                before_anchor: None,
            },
        }
    }

    fn scan_text(
        &mut self,
        text: &str,
        text_start: u64,
    ) -> io::Result<Option<DescriptorSearchMatch>> {
        self.scan_text_with_page_boundaries(text, text_start, true)
    }

    fn begin_page(&mut self, page_start: u64, page_number: usize) {
        self.position = DescriptorPosition {
            page_start,
            page_number,
            row: 0,
            col: 0,
        };
    }

    fn scan_fixed_page_text(&mut self, text: &str) -> io::Result<Option<DescriptorSearchMatch>> {
        self.scan_text_with_page_boundaries(text, self.position.page_start, false)
    }

    fn scan_text_with_page_boundaries(
        &mut self,
        text: &str,
        text_start: u64,
        advance_pages: bool,
    ) -> io::Result<Option<DescriptorSearchMatch>> {
        if text.is_empty() {
            return Ok(None);
        }
        let bytes = text.as_bytes();
        let stream_start = self.matcher.processed_bytes();
        self.coordinate_spans.push(DescriptorCoordinateSpan {
            stream_start,
            stream_end: stream_start + bytes.len(),
            position: self.position,
            text_start,
            advance_pages,
        });
        let stop_at_or_after = match (self.selection.direction, self.selection.anchor) {
            (SearchDirection::Forward, None) => Some(0),
            (SearchDirection::Forward, Some(anchor)) => anchor.checked_add(1),
            (SearchDirection::Backward, _) => None,
        };
        self.matcher.find_segment_matches(bytes, stop_at_or_after);

        let selected = consider_descriptor_candidates(
            &self.coordinate_spans,
            self.matcher.candidates(),
            self.matcher.overlap_start(),
            self.matcher.overlap(),
            stream_start,
            bytes,
            self.page_lines,
            &mut self.selection,
        )?;
        if selected.is_some() {
            return Ok(selected);
        }

        self.position = advance_descriptor_position(
            self.position,
            text,
            text_start,
            advance_pages,
            self.page_lines,
        );
        let retained_start = self.matcher.retained_start(bytes);
        trim_coordinate_spans(
            &mut self.coordinate_spans,
            retained_start,
            self.matcher.overlap_start(),
            self.matcher.overlap(),
            stream_start,
            bytes,
            self.page_lines,
        )?;
        self.matcher.commit_segment(bytes, retained_start);
        Ok(None)
    }

    fn finish(&self) -> Option<DescriptorSearchMatch> {
        self.selection.finish()
    }
}

#[allow(clippy::too_many_arguments)]
fn consider_descriptor_candidates(
    spans: &[DescriptorCoordinateSpan],
    candidates: &[usize],
    overlap_start: usize,
    overlap: &[u8],
    current_start: usize,
    current: &[u8],
    page_lines: usize,
    selection: &mut DescriptorSelection,
) -> io::Result<Option<DescriptorSearchMatch>> {
    let Some(&first_offset) = candidates.first() else {
        return Ok(None);
    };
    let last_offset = candidates[candidates.len() - 1];
    let match_at = |offset| {
        descriptor_search_match_at(
            spans,
            offset,
            overlap_start,
            overlap,
            current_start,
            current,
            page_lines,
        )
    };

    let Some(anchor) = selection.anchor else {
        let found = match_at(first_offset)?;
        selection.first_match = Some(found);
        selection.last_match = Some(found);
        return Ok(Some(found));
    };
    match selection.direction {
        SearchDirection::Forward => {
            let after_anchor = candidates.partition_point(|offset| *offset <= anchor);
            if let Some(&offset) = candidates.get(after_anchor) {
                return match_at(offset).map(Some);
            }
            if selection.first_match.is_none() {
                selection.first_match = Some(match_at(first_offset)?);
            }
        }
        SearchDirection::Backward => {
            let last_match = match_at(last_offset)?;
            let before_anchor = candidates.partition_point(|offset| *offset < anchor);
            if let Some(&offset) = before_anchor
                .checked_sub(1)
                .and_then(|index| candidates.get(index))
            {
                selection.before_anchor = Some(if offset == last_offset {
                    last_match
                } else {
                    match_at(offset)?
                });
            }
            selection.last_match = Some(last_match);
        }
    }
    Ok(None)
}

impl DescriptorSelection {
    fn finish(&self) -> Option<DescriptorSearchMatch> {
        match (self.anchor, self.direction) {
            (None, _) | (Some(_), SearchDirection::Forward) => self.first_match,
            (Some(_), SearchDirection::Backward) => self.before_anchor.or(self.last_match),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn descriptor_search_match_at(
    spans: &[DescriptorCoordinateSpan],
    offset: usize,
    overlap_start: usize,
    overlap: &[u8],
    current_start: usize,
    current: &[u8],
    page_lines: usize,
) -> io::Result<DescriptorSearchMatch> {
    descriptor_position_at(
        spans,
        offset,
        overlap_start,
        overlap,
        current_start,
        current,
        page_lines,
    )
    .map(|position| DescriptorSearchMatch {
        position,
        byte_offset: offset,
    })
}

fn descriptor_position_at(
    spans: &[DescriptorCoordinateSpan],
    offset: usize,
    overlap_start: usize,
    overlap: &[u8],
    current_start: usize,
    current: &[u8],
    page_lines: usize,
) -> io::Result<DescriptorPosition> {
    let span = spans
        .iter()
        .rev()
        .find(|span| span.stream_start <= offset && offset < span.stream_end)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "search coordinate span does not cover a literal match",
            )
        })?;
    let bytes = coordinate_bytes(
        *span,
        span.stream_start,
        offset,
        overlap_start,
        overlap,
        current_start,
        current,
    );
    let text = std::str::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(advance_descriptor_position(
        span.position,
        text,
        span.text_start,
        span.advance_pages,
        page_lines,
    ))
}

fn coordinate_bytes<'a>(
    span: DescriptorCoordinateSpan,
    start: usize,
    end: usize,
    overlap_start: usize,
    overlap: &'a [u8],
    current_start: usize,
    current: &'a [u8],
) -> &'a [u8] {
    if span.stream_start >= current_start {
        &current[start - current_start..end - current_start]
    } else {
        &overlap[start - overlap_start..end - overlap_start]
    }
}

fn trim_coordinate_spans(
    spans: &mut Vec<DescriptorCoordinateSpan>,
    retained_start: usize,
    overlap_start: usize,
    overlap: &[u8],
    current_start: usize,
    current: &[u8],
    page_lines: usize,
) -> io::Result<()> {
    let discarded = spans.partition_point(|span| span.stream_end <= retained_start);
    spans.drain(..discarded);
    let Some(first) = spans.first().copied() else {
        return Ok(());
    };
    if first.stream_start < retained_start {
        let position = descriptor_position_at(
            spans,
            retained_start,
            overlap_start,
            overlap,
            current_start,
            current,
            page_lines,
        )?;
        let delta = retained_start - first.stream_start;
        spans[0].stream_start = retained_start;
        spans[0].position = position;
        spans[0].text_start = first.text_start.saturating_add(delta as u64);
    }
    Ok(())
}

fn advance_descriptor_position(
    mut position: DescriptorPosition,
    text: &str,
    text_start: u64,
    advance_pages: bool,
    page_lines: usize,
) -> DescriptorPosition {
    let mut span_start = 0usize;
    for newline in memchr_iter(b'\n', text.as_bytes()) {
        position.col += scalar_count(&text[span_start..newline]);
        position.row += 1;
        position.col = 0;
        if advance_pages && position.row == page_lines {
            position.page_start = text_start.saturating_add(newline as u64 + 1);
            position.page_number += 1;
            position.row = 0;
        }
        span_start = newline + 1;
    }
    position.col += scalar_count(&text[span_start..]);
    position
}

fn scalar_count(text: &str) -> usize {
    if text.is_ascii() {
        text.len()
    } else {
        text.chars().count()
    }
}

#[cfg(test)]
mod tests;
