//! Purpose: scan a Huge-file descriptor once for UTF-8 and line metadata.
//! Owns: chunk-boundary UTF-8 validation, line starts, scalar counts, ASCII
//!   flags, and sparse scalar-column checkpoints.
//! Must not: reopen paths, retain the descriptor, render, edit, save, or know
//!   about App/terminal/Project/LLM policy.
//! Invariants: the first line starts at byte zero; metadata vectors describe
//!   every scanned line; checkpoints land on UTF-8 boundaries; total_bytes is
//!   the exact number of bytes read.
//! Phase: 2-bq shared optimized ASCII line scanning.

#[cfg(test)]
use std::fs::File;
#[cfg(test)]
use std::io::{self, Read};

#[cfg(test)]
use super::SCAN_CHUNK_BYTES;
use super::{LineCheckpoint, LINE_CHECKPOINT_INTERVAL_CHARS};

pub(crate) struct LineScan {
    pub(crate) line_starts: Vec<usize>,
    pub(crate) line_char_counts: Vec<usize>,
    pub(crate) line_is_ascii: Vec<bool>,
    pub(crate) line_checkpoints: Vec<LineCheckpoint>,
    pub(crate) line_checkpoint_starts: Vec<usize>,
    #[cfg(test)]
    pub(crate) total_bytes: usize,
}

pub(super) struct LineScanState {
    line_starts: Vec<usize>,
    line_char_counts: Vec<usize>,
    line_is_ascii: Vec<bool>,
    line_checkpoints: Vec<LineCheckpoint>,
    line_checkpoint_starts: Vec<usize>,
    current_line_chars: usize,
    current_line_is_ascii: bool,
}

impl LineScanState {
    pub(super) fn new(start_byte: usize) -> Self {
        Self {
            line_starts: vec![start_byte],
            line_char_counts: Vec::new(),
            line_is_ascii: Vec::new(),
            line_checkpoints: Vec::new(),
            line_checkpoint_starts: vec![0],
            current_line_chars: 0,
            current_line_is_ascii: true,
        }
    }

    pub(super) fn scan_valid_text(&mut self, text: &str, text_start_offset: usize) {
        if text.is_ascii() {
            self.scan_ascii_bytes(text.as_bytes(), text_start_offset);
            return;
        }

        for (byte_idx, ch) in text.char_indices() {
            if ch == '\n' {
                self.finish_line(text_start_offset + byte_idx + 1);
                continue;
            }
            if !ch.is_ascii() {
                self.current_line_is_ascii = false;
            }
            let next_col = self.current_line_chars + 1;
            if next_col.is_multiple_of(LINE_CHECKPOINT_INTERVAL_CHARS) {
                self.line_checkpoints.push(LineCheckpoint {
                    col: next_col,
                    byte_offset: text_start_offset + byte_idx + ch.len_utf8(),
                });
            }
            self.current_line_chars += 1;
        }
    }

    pub(super) fn scan_ascii_bytes(&mut self, bytes: &[u8], text_start_offset: usize) {
        let text = std::str::from_utf8(bytes).expect("ASCII bytes must be valid UTF-8");
        let mut segment_start = 0usize;
        for (newline_idx, _) in text.match_indices('\n') {
            self.push_ascii_checkpoints(
                text_start_offset + segment_start,
                newline_idx - segment_start,
            );
            self.current_line_chars += newline_idx - segment_start;
            self.finish_line(text_start_offset + newline_idx + 1);
            segment_start = newline_idx + 1;
        }
        self.push_ascii_checkpoints(
            text_start_offset + segment_start,
            bytes.len() - segment_start,
        );
        self.current_line_chars += bytes.len() - segment_start;
    }

    pub(super) fn finish_complete_page(&mut self) {
        self.line_starts.pop();
    }

    pub(super) fn finish_final_page(&mut self) {
        self.line_char_counts.push(self.current_line_chars);
        self.line_is_ascii.push(self.current_line_is_ascii);
        self.line_checkpoint_starts
            .push(self.line_checkpoints.len());
    }

    pub(super) fn into_scan(self, _total_bytes: usize) -> LineScan {
        LineScan {
            line_starts: self.line_starts,
            line_char_counts: self.line_char_counts,
            line_is_ascii: self.line_is_ascii,
            line_checkpoints: self.line_checkpoints,
            line_checkpoint_starts: self.line_checkpoint_starts,
            #[cfg(test)]
            total_bytes: _total_bytes,
        }
    }

    fn finish_line(&mut self, next_line_start: usize) {
        self.line_char_counts.push(self.current_line_chars);
        self.line_is_ascii.push(self.current_line_is_ascii);
        self.line_checkpoint_starts
            .push(self.line_checkpoints.len());
        self.current_line_chars = 0;
        self.current_line_is_ascii = true;
        self.line_starts.push(next_line_start);
    }

    fn push_ascii_checkpoints(&mut self, segment_start_offset: usize, segment_len: usize) {
        push_ascii_line_checkpoints(
            &mut self.line_checkpoints,
            self.current_line_chars,
            segment_start_offset,
            segment_len,
        );
    }
}

#[cfg(test)]
pub(crate) fn scan_utf8_lines(file: &mut File) -> io::Result<LineScan> {
    let mut lines = LineScanState::new(0);
    let mut carry: Vec<u8> = Vec::new();
    let mut offset = 0usize;
    let mut chunk = vec![0u8; SCAN_CHUNK_BYTES];

    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        let bytes = &chunk[..n];
        let carry_len = carry.len();
        let text_start_offset = offset - carry_len;

        let mut combined;
        let text_bytes = if carry.is_empty() {
            bytes
        } else {
            combined = Vec::with_capacity(carry.len() + bytes.len());
            combined.extend_from_slice(&carry);
            combined.extend_from_slice(bytes);
            carry.clear();
            &combined
        };

        let valid_end = match std::str::from_utf8(text_bytes) {
            Ok(_) => text_bytes.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        };
        let valid_text = std::str::from_utf8(&text_bytes[..valid_end])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        lines.scan_valid_text(valid_text, text_start_offset);
        if valid_end < text_bytes.len() {
            carry.extend_from_slice(&text_bytes[valid_end..]);
        }
        offset += n;
    }

    if !carry.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete utf-8 sequence at end of file",
        ));
    }

    lines.finish_final_page();
    Ok(lines.into_scan(offset))
}

fn push_ascii_line_checkpoints(
    line_checkpoints: &mut Vec<LineCheckpoint>,
    current_col: usize,
    segment_start_offset: usize,
    segment_len: usize,
) {
    if segment_len == 0 {
        return;
    }
    let segment_end_col = current_col + segment_len;
    let mut next_col =
        ((current_col / LINE_CHECKPOINT_INTERVAL_CHARS) + 1) * LINE_CHECKPOINT_INTERVAL_CHARS;

    while next_col <= segment_end_col {
        line_checkpoints.push(LineCheckpoint {
            col: next_col,
            byte_offset: segment_start_offset + (next_col - current_col),
        });
        next_col += LINE_CHECKPOINT_INTERVAL_CHARS;
    }
}
