//! Purpose: search a stable file descriptor without blocking the terminal loop.
//! Owns: explicit worker lifetime, cancellation, chunked UTF-8 validation, and
//!   cross-chunk matching with page/row/scalar-column tracking.
//! Must not: render, mutate App/Buffer state, reopen paths, index projects, or network.
//! Invariants: descriptor bytes are processed once with bounded memory; matches
//!   can cross read boundaries; result positions use configured logical-line pages.
//! Phase: 2-bo whole-file paged Ctrl+F.

use std::io;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crate::buffer::{Buffer, Cursor, DescriptorPosition, DescriptorSource};
const SEARCH_CHUNK_BYTES: usize = 64 * 1024;

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
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => None,
        }
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
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let result = scan_descriptor(source, &query, &worker_cancel)
            .unwrap_or_else(|error| SearchResult::Error(error.to_string()));
        if !worker_cancel.load(Ordering::Acquire) {
            let _ = sender.send(result);
        }
    });
    SearchTask { receiver, cancel }
}

pub(crate) fn find_first(buffer: &dyn Buffer, query: &str) -> Option<Cursor> {
    if query.is_empty() {
        return None;
    }
    for row in 0..buffer.line_count() {
        let line = buffer.line(row)?;
        if let Some(byte_col) = line.find(query) {
            return Some(Cursor {
                row,
                col: line[..byte_col].chars().count(),
            });
        }
    }
    None
}

fn scan_descriptor(
    source: DescriptorSource,
    query: &str,
    cancel: &AtomicBool,
) -> io::Result<SearchResult> {
    if query.is_empty() || query.contains('\n') {
        return Ok(SearchResult::NotFound);
    }
    let initial_meta = source.file.metadata()?;
    if initial_meta.len() != source.total_bytes {
        return Err(changed_file_error());
    }
    let initial_modified = initial_meta.modified().ok();
    let mut scanner = Scanner::new(query, source.page_lines);
    let mut chunk = vec![0u8; SEARCH_CHUNK_BYTES];
    let mut carry = Vec::new();
    let mut offset = 0u64;
    while offset < source.total_bytes {
        if cancel.load(Ordering::Acquire) {
            return Ok(SearchResult::NotFound);
        }
        let read = source.file.read_at(&mut chunk, offset)?;
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
    Ok(SearchResult::NotFound)
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
    query_chars: usize,
    page_lines: usize,
    page_start: u64,
    page_number: usize,
    row: usize,
    col: usize,
}

impl Scanner {
    fn new(query: &str, page_lines: usize) -> Self {
        Self {
            query: query.as_bytes().to_vec(),
            prefix: prefix_table(query.as_bytes()),
            matched: 0,
            query_chars: query.chars().count(),
            page_lines,
            page_start: 0,
            page_number: 1,
            row: 0,
            col: 0,
        }
    }

    fn scan_text(&mut self, text: &str, text_start: u64) -> Option<DescriptorPosition> {
        for (byte_index, ch) in text.char_indices() {
            let mut encoded = [0; 4];
            let mut found = false;
            for byte in ch.encode_utf8(&mut encoded).as_bytes() {
                found |= self.feed(*byte);
            }
            if ch == '\n' {
                self.row += 1;
                self.col = 0;
                if self.row == self.page_lines {
                    self.page_start = text_start + byte_index as u64 + 1;
                    self.page_number += 1;
                    self.row = 0;
                }
            } else {
                self.col += 1;
            }
            if found {
                return Some(DescriptorPosition {
                    page_start: self.page_start,
                    page_number: self.page_number,
                    row: self.row,
                    col: self.col.saturating_sub(self.query_chars),
                });
            }
        }
        None
    }

    fn feed(&mut self, byte: u8) -> bool {
        while self.matched > 0 && self.query[self.matched] != byte {
            self.matched = self.prefix[self.matched - 1];
        }
        if self.query[self.matched] == byte {
            self.matched += 1;
        }
        if self.matched == self.query.len() {
            self.matched = self.prefix[self.matched - 1];
            true
        } else {
            false
        }
    }
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
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn scan_text_file(text: &[u8], query: &str, page_lines: usize) -> SearchResult {
        let path = std::env::temp_dir().join(format!(
            "catomic_search_scan_{}_{}.txt",
            std::process::id(),
            text.len()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, text).unwrap();
        let source = DescriptorSource {
            file: std::fs::File::open(&path).unwrap(),
            total_bytes: text.len() as u64,
            page_lines,
        };
        let result = scan_descriptor(source, query, &AtomicBool::new(false)).unwrap();
        let _ = std::fs::remove_file(path);
        result
    }

    #[test]
    fn descriptor_match_crosses_read_chunk_boundary() {
        let prefix = "a".repeat(SEARCH_CHUNK_BYTES - 3);
        let text = format!("{prefix}needle tail");

        let SearchResult::Found(position) = scan_text_file(text.as_bytes(), "needle", 20_000)
        else {
            panic!("expected cross-boundary match");
        };

        assert_eq!(position.page_number, 1);
        assert_eq!(position.row, 0);
        assert_eq!(position.col, SEARCH_CHUNK_BYTES - 3);
    }

    #[test]
    fn descriptor_match_tracks_unicode_scalar_column_and_page() {
        let SearchResult::Found(position) =
            scan_text_file("α\nβ\nγ needle".as_bytes(), "needle", 1)
        else {
            panic!("expected Unicode match");
        };

        assert_eq!(position.page_number, 3);
        assert_eq!(position.row, 0);
        assert_eq!(position.col, 2);
        assert_eq!(position.page_start, "α\nβ\n".len() as u64);
    }
}
