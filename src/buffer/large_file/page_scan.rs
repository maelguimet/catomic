//! Purpose: scan one configured line page from a Huge-file descriptor.
//! Owns: bounded page boundaries, chunk-boundary UTF-8 validation, and page metadata.
//! Must not: reopen paths, retain descriptors, render, edit, or choose App policy.
//! Invariants: a complete non-final page ends after its configured newline count;
//!   page metadata uses absolute descriptor offsets and retains no file content.
//! Phase: 2-bl configurable paged Huge-file foundation.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

use super::scan::{scan_valid_text_lines, LineScan};
use super::{LineCheckpoint, SCAN_CHUNK_BYTES};

pub(super) struct PageScan {
    pub(super) lines: LineScan,
    pub(super) start_byte: usize,
    pub(super) end_byte: usize,
    pub(super) next_page_start: Option<usize>,
}

pub(super) fn scan_utf8_page(
    file: &File,
    start_byte: usize,
    page_lines: usize,
) -> io::Result<PageScan> {
    if page_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "page line count must be positive",
        ));
    }
    let mut state = PageScanState::new(start_byte, page_lines);
    let mut chunk = vec![0u8; SCAN_CHUNK_BYTES];
    loop {
        let n = file.read_at(&mut chunk, state.offset as u64)?;
        if n == 0 {
            break;
        }
        let (used, page_complete) = bytes_through_newline(&chunk[..n], state.lines_remaining);
        state.scan_chunk(&chunk[..used])?;
        if page_complete {
            state.finish_complete_page()?;
            let next_page_start = state.offset;
            return Ok(state.into_scan(Some(next_page_start)));
        }
    }
    state.finish_final_page()?;
    Ok(state.into_scan(None))
}

struct PageScanState {
    start_byte: usize,
    offset: usize,
    lines_remaining: usize,
    line_starts: Vec<usize>,
    line_char_counts: Vec<usize>,
    line_is_ascii: Vec<bool>,
    line_checkpoints: Vec<LineCheckpoint>,
    line_checkpoint_starts: Vec<usize>,
    current_line_chars: usize,
    current_line_is_ascii: bool,
    carry: Vec<u8>,
}

impl PageScanState {
    fn new(start_byte: usize, page_lines: usize) -> Self {
        Self {
            start_byte,
            offset: start_byte,
            lines_remaining: page_lines,
            line_starts: vec![start_byte],
            line_char_counts: Vec::new(),
            line_is_ascii: Vec::new(),
            line_checkpoints: Vec::new(),
            line_checkpoint_starts: vec![0],
            current_line_chars: 0,
            current_line_is_ascii: true,
            carry: Vec::new(),
        }
    }

    fn scan_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        let newline_count = bytes.iter().filter(|byte| **byte == b'\n').count();
        self.lines_remaining = self.lines_remaining.saturating_sub(newline_count);
        let carry_len = self.carry.len();
        let text_start_offset = self.offset - carry_len;
        let mut combined;
        let text_bytes = if self.carry.is_empty() {
            bytes
        } else {
            combined = Vec::with_capacity(carry_len + bytes.len());
            combined.extend_from_slice(&self.carry);
            combined.extend_from_slice(bytes);
            self.carry.clear();
            &combined
        };
        let valid_end = valid_utf8_end(text_bytes)?;
        let valid_text = std::str::from_utf8(&text_bytes[..valid_end])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        scan_valid_text_lines(
            valid_text,
            text_start_offset,
            &mut self.line_starts,
            &mut self.line_char_counts,
            &mut self.line_is_ascii,
            &mut self.line_checkpoints,
            &mut self.line_checkpoint_starts,
            &mut self.current_line_chars,
            &mut self.current_line_is_ascii,
        );
        self.carry.extend_from_slice(&text_bytes[valid_end..]);
        self.offset += bytes.len();
        Ok(())
    }

    fn finish_complete_page(&mut self) -> io::Result<()> {
        if !self.carry.is_empty() {
            return Err(incomplete_utf8_error());
        }
        self.line_starts.pop();
        Ok(())
    }

    fn finish_final_page(&mut self) -> io::Result<()> {
        if !self.carry.is_empty() {
            return Err(incomplete_utf8_error());
        }
        self.line_char_counts.push(self.current_line_chars);
        self.line_is_ascii.push(self.current_line_is_ascii);
        self.line_checkpoint_starts
            .push(self.line_checkpoints.len());
        Ok(())
    }

    fn into_scan(self, next_page_start: Option<usize>) -> PageScan {
        PageScan {
            lines: LineScan {
                line_starts: self.line_starts,
                line_char_counts: self.line_char_counts,
                line_is_ascii: self.line_is_ascii,
                line_checkpoints: self.line_checkpoints,
                line_checkpoint_starts: self.line_checkpoint_starts,
                total_bytes: self.offset - self.start_byte,
            },
            start_byte: self.start_byte,
            end_byte: self.offset,
            next_page_start,
        }
    }
}

fn bytes_through_newline(bytes: &[u8], remaining: usize) -> (usize, bool) {
    let mut seen = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            seen += 1;
            if seen == remaining {
                return (index + 1, true);
            }
        }
    }
    (bytes.len(), false)
}

fn valid_utf8_end(bytes: &[u8]) -> io::Result<usize> {
    match std::str::from_utf8(bytes) {
        Ok(_) => Ok(bytes.len()),
        Err(error) if error.error_len().is_none() => Ok(error.valid_up_to()),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}

fn incomplete_utf8_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "incomplete utf-8 sequence at end of file",
    )
}
