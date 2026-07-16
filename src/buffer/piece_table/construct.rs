//! PieceTable constructors and input normalization.
//!
//! Purpose: keep construction paths for new, borrowed text, owned open buffers,
//!   and scanned file-backed originals
//!   out of the main piece_table module.
//! Owns: PieceTable::new, PieceTable::from_text, PieceTable::from_owned_text,
//!   and PieceTable::from_file.
//! Must not: perform edits, undo/redo, queries, UI, Project, or LLM work.
//! Invariants: CRLF/CR normalize to LF; LF-only owned input moves into original
//!   without cloning; cursor starts at (0,0); initial piece/index/piece_starts are consistent.
//! Phase: 2-bi file-backed PieceTable foundation.

use std::fs::File;
use std::io;
use std::path::Path;

use crate::buffer::large_file::scan::scan_utf8_lines;
use crate::buffer::line_index::LineIndex;
use crate::buffer::Cursor;

use super::types::{FileMetadataSnapshot, OriginalBacking, Piece, PieceTable, Source};

impl PieceTable {
    pub fn new() -> Self {
        let pieces = vec![Piece {
            source: Source::Original,
            start: 0,
            len: 0,
        }];
        let original = OriginalBacking::empty();
        let index = LineIndex::from_text("");
        let piece_starts = vec![0];
        Self {
            original,
            add: String::new(),
            pieces,
            index,
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
            piece_starts,
            undo_stack: crate::buffer::undo::UndoStack::new(),
            recording: true,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let normalized = if text.as_bytes().contains(&b'\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.to_string()
        };
        Self::from_normalized_text(normalized)
    }

    /// Build from an owned read buffer so open paths do not clone large files.
    /// Still normalizes CRLF/CR inputs; LF-only inputs move directly into original.
    pub(crate) fn from_owned_text(text: String) -> Self {
        let normalized = if text.as_bytes().contains(&b'\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text
        };
        Self::from_normalized_text(normalized)
    }

    pub(crate) fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let snapshot = FileMetadataSnapshot::capture(&file)?;
        let scan = scan_utf8_lines(&mut file)?;
        if FileMetadataSnapshot::capture(&file)? != snapshot {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-backed original changed while scanning",
            ));
        }
        let newline_offsets = scan
            .line_starts
            .iter()
            .skip(1)
            .map(|start| start - 1)
            .collect();
        let pieces = vec![Piece {
            source: Source::Original,
            start: 0,
            len: scan.total_bytes,
        }];
        Ok(Self {
            original: OriginalBacking::from_file(file, snapshot, newline_offsets),
            add: String::new(),
            pieces,
            index: LineIndex {
                line_starts: scan.line_starts,
                total_bytes: scan.total_bytes,
            },
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
            piece_starts: vec![0],
            undo_stack: crate::buffer::undo::UndoStack::new(),
            recording: true,
        })
    }

    fn from_normalized_text(normalized: String) -> Self {
        let index = LineIndex::from_text(&normalized);
        let (original, pieces) = if normalized.is_empty() {
            (
                OriginalBacking::empty(),
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len: 0,
                }],
            )
        } else {
            let len = normalized.len();
            (
                OriginalBacking::from_owned(normalized),
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len,
                }],
            )
        };
        let piece_starts = vec![0];
        Self {
            original,
            add: String::new(),
            pieces,
            index,
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
            piece_starts,
            undo_stack: crate::buffer::undo::UndoStack::new(),
            recording: true,
        }
    }
}
