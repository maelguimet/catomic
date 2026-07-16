//! Purpose: scan a Huge-file descriptor once for UTF-8 and line metadata.
//! Owns: chunk-boundary UTF-8 validation, line starts, scalar counts, ASCII
//!   flags, and sparse scalar-column checkpoints.
//! Must not: reopen paths, retain the descriptor, render, edit, save, or know
//!   about App/terminal/Project/LLM policy.
//! Invariants: the first line starts at byte zero; metadata vectors describe
//!   every scanned line; checkpoints land on UTF-8 boundaries; total_bytes is
//!   the exact number of bytes read.
//! Phase: 2-bg LargeFileBuffer size-hygiene split.

use std::fs::File;
use std::io::{self, Read};

use super::{LineCheckpoint, LINE_CHECKPOINT_INTERVAL_CHARS, SCAN_CHUNK_BYTES};

pub(super) struct LineScan {
    pub(super) line_starts: Vec<usize>,
    pub(super) line_char_counts: Vec<usize>,
    pub(super) line_is_ascii: Vec<bool>,
    pub(super) line_checkpoints: Vec<LineCheckpoint>,
    pub(super) line_checkpoint_starts: Vec<usize>,
    pub(super) total_bytes: usize,
}

pub(super) fn scan_utf8_lines(file: &mut File) -> io::Result<LineScan> {
    let mut line_starts = vec![0usize];
    let mut line_char_counts = Vec::new();
    let mut line_is_ascii = Vec::new();
    let mut line_checkpoints = Vec::new();
    let mut line_checkpoint_starts = vec![0usize];
    let mut current_line_chars = 0usize;
    let mut current_line_is_ascii = true;
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
            &mut line_is_ascii,
            &mut line_checkpoints,
            &mut line_checkpoint_starts,
            &mut current_line_chars,
            &mut current_line_is_ascii,
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
    line_is_ascii.push(current_line_is_ascii);
    line_checkpoint_starts.push(line_checkpoints.len());
    Ok(LineScan {
        line_starts,
        line_char_counts,
        line_is_ascii,
        line_checkpoints,
        line_checkpoint_starts,
        total_bytes: offset,
    })
}

fn scan_valid_text_lines(
    text: &str,
    text_start_offset: usize,
    line_starts: &mut Vec<usize>,
    line_char_counts: &mut Vec<usize>,
    line_is_ascii: &mut Vec<bool>,
    line_checkpoints: &mut Vec<LineCheckpoint>,
    line_checkpoint_starts: &mut Vec<usize>,
    current_line_chars: &mut usize,
    current_line_is_ascii: &mut bool,
) {
    if text.is_ascii() {
        let mut segment_start = 0usize;
        for (newline_idx, _) in text.match_indices('\n') {
            push_ascii_line_checkpoints(
                line_checkpoints,
                *current_line_chars,
                text_start_offset + segment_start,
                newline_idx - segment_start,
            );
            *current_line_chars += newline_idx - segment_start;
            line_char_counts.push(*current_line_chars);
            line_is_ascii.push(*current_line_is_ascii);
            line_checkpoint_starts.push(line_checkpoints.len());
            *current_line_chars = 0;
            *current_line_is_ascii = true;
            line_starts.push(text_start_offset + newline_idx + 1);
            segment_start = newline_idx + 1;
        }
        push_ascii_line_checkpoints(
            line_checkpoints,
            *current_line_chars,
            text_start_offset + segment_start,
            text.len() - segment_start,
        );
        *current_line_chars += text.len() - segment_start;
        return;
    }

    for (byte_idx, ch) in text.char_indices() {
        if ch == '\n' {
            line_char_counts.push(*current_line_chars);
            line_is_ascii.push(*current_line_is_ascii);
            line_checkpoint_starts.push(line_checkpoints.len());
            *current_line_chars = 0;
            *current_line_is_ascii = true;
            line_starts.push(text_start_offset + byte_idx + 1);
        } else {
            if !ch.is_ascii() {
                *current_line_is_ascii = false;
            }
            let next_col = *current_line_chars + 1;
            if next_col % LINE_CHECKPOINT_INTERVAL_CHARS == 0 {
                line_checkpoints.push(LineCheckpoint {
                    col: next_col,
                    byte_offset: text_start_offset + byte_idx + ch.len_utf8(),
                });
            }
            *current_line_chars += 1;
        }
    }
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
