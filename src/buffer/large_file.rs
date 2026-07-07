//! Purpose: provide a read-only, file-backed Buffer for Huge files in limited mode.
//! Owns: chunked UTF-8/newline scan, ranged visible-line reads, read-only movement.
//! Must not: edit file content, perform writes, own App policy, construct watchers,
//!   depend on Project/LLM, or materialize the whole file for rendering/navigation.
//! Invariants: line_starts[0] == 0; line_char_counts.len() == line_starts.len();
//!   file content was UTF-8 valid at construction; cursor row/col stays clamped.
//! Phase: 2B limited Huge-file storage foundation.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Cursor, LineView};

const SCAN_CHUNK_BYTES: usize = 64 * 1024;

/// Read-only file-backed buffer for limited Huge-file mode.
pub(crate) struct LargeFileBuffer {
    path: PathBuf,
    file: File,
    line_starts: Vec<usize>,
    line_char_counts: Vec<usize>,
    total_bytes: usize,
    cursor: Cursor,
}

impl LargeFileBuffer {
    pub(crate) fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut scan_file = File::open(&path)?;
        let scan = scan_utf8_lines(&mut scan_file)?;
        let file = File::open(&path)?;
        Ok(Self {
            path,
            file,
            line_starts: scan.line_starts,
            line_char_counts: scan.line_char_counts,
            total_bytes: scan.total_bytes,
            cursor: Cursor { row: 0, col: 0 },
        })
    }

    fn line_start_byte(&self, row: usize) -> usize {
        self.line_starts[row.min(self.line_starts.len().saturating_sub(1))]
    }

    fn line_end_byte(&self, row: usize) -> usize {
        let row = row.min(self.line_starts.len().saturating_sub(1));
        if row + 1 < self.line_starts.len() {
            self.line_starts[row + 1].saturating_sub(1)
        } else {
            self.total_bytes
        }
    }

    fn read_range_bytes(&self, start: usize, end: usize) -> io::Result<Vec<u8>> {
        if start >= end {
            return Ok(Vec::new());
        }
        let mut out = vec![0u8; end - start];
        let mut filled = 0usize;
        while filled < out.len() {
            let n = self
                .file
                .read_at(&mut out[filled..], (start + filled) as u64)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read from large file buffer",
                ));
            }
            filled += n;
        }
        Ok(out)
    }

    fn read_range_to_string(&self, start: usize, end: usize) -> io::Result<String> {
        let bytes = self.read_range_bytes(start, end)?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn line_to_string(&self, row: usize) -> io::Result<String> {
        if row >= self.line_starts.len() {
            return Ok(String::new());
        }
        self.read_range_to_string(self.line_start_byte(row), self.line_end_byte(row))
    }

    fn line_window_to_string(&self, row: usize, start_col: usize, width: usize) -> String {
        if row >= self.line_starts.len() || width == 0 {
            return String::new();
        }
        let line_chars = self.line_char_counts[row];
        if start_col >= line_chars {
            return String::new();
        }
        self.read_line_window(row, start_col, width)
            .unwrap_or_default()
    }

    fn read_line_window(&self, row: usize, start_col: usize, width: usize) -> io::Result<String> {
        let mut pos = self.line_start_byte(row);
        let end = self.line_end_byte(row);
        let mut carry: Vec<u8> = Vec::new();
        let mut seen = 0usize;
        let mut taken = 0usize;
        let mut out = String::new();

        while pos < end && taken < width {
            let chunk_end = (pos + SCAN_CHUNK_BYTES).min(end);
            let mut bytes = self.read_range_bytes(pos, chunk_end)?;
            pos = chunk_end;

            if !carry.is_empty() {
                let mut joined = Vec::with_capacity(carry.len() + bytes.len());
                joined.extend_from_slice(&carry);
                joined.extend_from_slice(&bytes);
                bytes = joined;
                carry.clear();
            }

            let valid_end = match std::str::from_utf8(&bytes) {
                Ok(_) => bytes.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };
            let text = std::str::from_utf8(&bytes[..valid_end])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            for ch in text.chars() {
                if seen >= start_col && taken < width {
                    out.push(ch);
                    taken += 1;
                }
                seen += 1;
                if taken >= width {
                    break;
                }
            }
            if valid_end < bytes.len() {
                carry.extend_from_slice(&bytes[valid_end..]);
            }
        }

        if taken < width && !carry.is_empty() {
            let text = std::str::from_utf8(&carry)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            for ch in text.chars() {
                if seen >= start_col && taken < width {
                    out.push(ch);
                    taken += 1;
                }
                seen += 1;
                if taken >= width {
                    break;
                }
            }
        }

        Ok(out)
    }
}

impl Buffer for LargeFileBuffer {
    fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        if row >= self.line_starts.len() {
            return None;
        }
        Some(Cow::Owned(self.line_to_string(row).unwrap_or_default()))
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let end = (start + height).min(self.line_count());
        (start..end)
            .map(|row| LineView {
                content: self.line_to_string(row).unwrap_or_default(),
            })
            .collect()
    }

    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView> {
        let end = (start + height).min(self.line_count());
        (start..end)
            .map(|row| LineView {
                content: self.line_window_to_string(row, start_col, width),
            })
            .collect()
    }

    fn line_char_count(&self, row: usize) -> Option<usize> {
        self.line_char_counts.get(row).copied()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn to_string(&self) -> String {
        crate::file::io::read_to_string(&self.path).unwrap_or_default()
    }

    fn lines(&self) -> Vec<String> {
        (0..self.line_count())
            .map(|row| self.line_to_string(row).unwrap_or_default())
            .collect()
    }

    fn insert_char(&mut self, _ch: char) {}

    fn insert_newline(&mut self) {}

    fn delete_back(&mut self) {}

    fn delete_forward(&mut self) {}

    fn move_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.line_char_counts[self.cursor.row];
        }
    }

    fn move_right(&mut self) {
        if self.cursor.col < self.line_char_counts[self.cursor.row] {
            self.cursor.col += 1;
        } else if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.cursor.col.min(self.line_char_counts[self.cursor.row]);
        }
    }

    fn move_down(&mut self) {
        if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = self.cursor.col.min(self.line_char_counts[self.cursor.row]);
        }
    }

    fn undo(&mut self) {}

    fn redo(&mut self) {}

    fn edit_history_position(&self) -> u64 {
        0
    }
}

struct LineScan {
    line_starts: Vec<usize>,
    line_char_counts: Vec<usize>,
    total_bytes: usize,
}

fn scan_utf8_lines(file: &mut File) -> io::Result<LineScan> {
    let mut line_starts = vec![0usize];
    let mut line_char_counts = Vec::new();
    let mut current_line_chars = 0usize;
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
        scan_valid_text_lines(
            valid_text,
            text_start_offset,
            &mut line_starts,
            &mut line_char_counts,
            &mut current_line_chars,
        );
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

    line_char_counts.push(current_line_chars);
    Ok(LineScan {
        line_starts,
        line_char_counts,
        total_bytes: offset,
    })
}

fn scan_valid_text_lines(
    text: &str,
    text_start_offset: usize,
    line_starts: &mut Vec<usize>,
    line_char_counts: &mut Vec<usize>,
    current_line_chars: &mut usize,
) {
    if text.is_ascii() {
        let mut segment_start = 0usize;
        for (newline_idx, _) in text.match_indices('\n') {
            *current_line_chars += newline_idx - segment_start;
            line_char_counts.push(*current_line_chars);
            *current_line_chars = 0;
            line_starts.push(text_start_offset + newline_idx + 1);
            segment_start = newline_idx + 1;
        }
        *current_line_chars += text.len() - segment_start;
        return;
    }

    for (byte_idx, ch) in text.char_indices() {
        if ch == '\n' {
            line_char_counts.push(*current_line_chars);
            *current_line_chars = 0;
            line_starts.push(text_start_offset + byte_idx + 1);
        } else {
            *current_line_chars += 1;
        }
    }
}

#[cfg(test)]
#[path = "large_file_tests.rs"]
mod tests;
