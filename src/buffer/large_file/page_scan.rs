//! Purpose: scan one configured line page from a Huge-file descriptor.
//! Owns: bounded page boundaries, chunk-boundary UTF-8 validation, and page metadata.
//! Must not: reopen paths, retain descriptors, render, edit, or choose App policy.
//! Invariants: a complete non-final page ends after its configured newline count;
//!   page metadata uses absolute descriptor offsets and retains no file content.
//! Phase: 2-bq optimized ASCII paged-file scanning.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

use super::scan::{scan_ascii_bytes_lines, scan_valid_text_lines, LineScan};
use super::{LineCheckpoint, SCAN_CHUNK_BYTES};

pub(crate) struct PageScan {
    pub(crate) lines: LineScan,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) next_page_start: Option<usize>,
}

pub(crate) fn scan_utf8_page(
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
        let page_chunk = page_chunk(&chunk[..n], state.lines_remaining);
        state.scan_chunk(
            &chunk[..page_chunk.used],
            page_chunk.newline_count,
            page_chunk.is_ascii,
        )?;
        if page_chunk.page_complete {
            state.finish_complete_page()?;
            let next_page_start = state.offset;
            return Ok(state.into_scan(Some(next_page_start)));
        }
    }
    state.finish_final_page()?;
    Ok(state.into_scan(None))
}

pub(crate) fn find_previous_page_start(
    file: &File,
    current_start: usize,
    page_lines: usize,
) -> io::Result<usize> {
    let target_newline = page_lines.saturating_add(1);
    let mut seen = 0usize;
    let mut end = current_start;
    let mut chunk = vec![0u8; SCAN_CHUNK_BYTES];
    while end > 0 {
        let start = end.saturating_sub(chunk.len());
        let len = end - start;
        read_exact_at(file, &mut chunk[..len], start)?;
        for index in (0..len).rev() {
            if chunk[index] == b'\n' {
                seen += 1;
                if seen == target_newline {
                    return Ok(start + index + 1);
                }
            }
        }
        end = start;
    }
    Ok(0)
}

fn read_exact_at(file: &File, mut out: &mut [u8], mut offset: usize) -> io::Result<()> {
    while !out.is_empty() {
        let read = file.read_at(out, offset as u64)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short read while locating previous file page",
            ));
        }
        offset += read;
        out = &mut out[read..];
    }
    Ok(())
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

    fn scan_chunk(&mut self, bytes: &[u8], newline_count: usize, is_ascii: bool) -> io::Result<()> {
        self.lines_remaining = self.lines_remaining.saturating_sub(newline_count);
        let carry_len = self.carry.len();
        let text_start_offset = self.offset - carry_len;
        if self.carry.is_empty() && is_ascii {
            self.scan_ascii_bytes(bytes, text_start_offset);
            self.offset += bytes.len();
            return Ok(());
        }
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

    fn scan_ascii_bytes(&mut self, bytes: &[u8], text_start_offset: usize) {
        scan_ascii_bytes_lines(
            bytes,
            text_start_offset,
            &mut self.line_starts,
            &mut self.line_char_counts,
            &mut self.line_is_ascii,
            &mut self.line_checkpoints,
            &mut self.line_checkpoint_starts,
            &mut self.current_line_chars,
            &mut self.current_line_is_ascii,
        );
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

struct PageChunk {
    used: usize,
    newline_count: usize,
    page_complete: bool,
    is_ascii: bool,
}

fn page_chunk(bytes: &[u8], remaining: usize) -> PageChunk {
    if bytes.is_ascii() {
        return ascii_page_chunk(bytes, remaining);
    }
    non_ascii_page_chunk(bytes, remaining)
}

fn ascii_page_chunk(bytes: &[u8], remaining: usize) -> PageChunk {
    let text = std::str::from_utf8(bytes).expect("ASCII bytes must be valid UTF-8");
    let mut seen = 0usize;
    for (index, _) in text.match_indices('\n') {
        seen += 1;
        if seen == remaining {
            return PageChunk {
                used: index + 1,
                newline_count: seen,
                page_complete: true,
                is_ascii: true,
            };
        }
    }
    PageChunk {
        used: bytes.len(),
        newline_count: seen,
        page_complete: false,
        is_ascii: true,
    }
}

fn non_ascii_page_chunk(bytes: &[u8], remaining: usize) -> PageChunk {
    let mut seen = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            seen += 1;
            if seen == remaining {
                return PageChunk {
                    used: index + 1,
                    newline_count: seen,
                    page_complete: true,
                    is_ascii: false,
                };
            }
        }
    }
    PageChunk {
        used: bytes.len(),
        newline_count: seen,
        page_complete: false,
        is_ascii: false,
    }
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
