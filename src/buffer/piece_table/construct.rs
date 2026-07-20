//! PieceTable constructors and input normalization.
//!
//! Purpose: keep construction paths for new, borrowed text, owned open buffers,
//!   and scanned file-backed originals
//!   out of the main piece_table module.
//! Owns: PieceTable::new, text constructors, and whole-file/page-range
//!   descriptor-backed constructors.
//! Must not: perform edits, undo/redo, queries, UI, Project, or LLM work.
//! Invariants: CRLF/CR normalize to LF; LF-only owned input moves into original
//!   without cloning; cursor starts at (0,0); initial piece/index/piece_starts are consistent.

use std::fs::File;
use std::io;
#[cfg(test)]
use std::path::Path;

use crate::buffer::large_file::page_scan::scan_utf8_page;
#[cfg(test)]
use crate::buffer::large_file::scan::scan_utf8_lines;
use crate::buffer::line_index::LineIndex;
use crate::buffer::Cursor;

use super::file_original::{FileMetadataSnapshot, FileOriginalMetadata};
use super::types::{OriginalBacking, Piece, PieceTable, Source};

pub(crate) struct FileBackedPage {
    pub(crate) buffer: PieceTable,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) next_page_start: Option<usize>,
    #[cfg(test)]
    pub(crate) total_bytes: usize,
}

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

    #[cfg(test)]
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
        let total_bytes = scan.total_bytes;
        Ok(Self::from_file_scan(file, snapshot, scan, 0, total_bytes))
    }

    pub(crate) fn from_file_page(
        file: File,
        start_byte: usize,
        page_lines: usize,
    ) -> io::Result<FileBackedPage> {
        let snapshot = FileMetadataSnapshot::capture(&file)?;
        let total_bytes = usize::try_from(snapshot.len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file size exceeds this platform's addressable range",
            )
        })?;
        if start_byte > total_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "page start exceeds file length",
            ));
        }
        let page = scan_utf8_page(&file, start_byte, page_lines)?;
        if FileMetadataSnapshot::capture(&file)? != snapshot {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-backed original changed while scanning page",
            ));
        }
        let mut scan = page.lines;
        if page.next_page_start.is_some() {
            scan.line_starts.push(page.end_byte);
            scan.line_char_counts.push(0);
            scan.line_is_ascii.push(true);
            scan.line_checkpoint_starts
                .push(scan.line_checkpoints.len());
        }
        Ok(FileBackedPage {
            buffer: Self::from_file_scan(file, snapshot, scan, page.start_byte, page.end_byte),
            start_byte: page.start_byte,
            end_byte: page.end_byte,
            next_page_start: page.next_page_start,
            #[cfg(test)]
            total_bytes,
        })
    }

    fn from_file_scan(
        file: File,
        snapshot: FileMetadataSnapshot,
        scan: crate::buffer::large_file::scan::LineScan,
        range_start: usize,
        range_end: usize,
    ) -> Self {
        let local_line_starts = scan
            .line_starts
            .iter()
            .map(|start| {
                let removed = scan.crlf_offsets.partition_point(|offset| offset < start);
                start - range_start - removed
            })
            .collect();
        let newline_offsets = scan
            .line_starts
            .iter()
            .skip(1)
            .map(|start| start - 1)
            .collect();
        let original_metadata = FileOriginalMetadata {
            range_start,
            range_end,
            newline_offsets,
            line_char_counts: scan.line_char_counts,
            line_is_ascii: scan.line_is_ascii,
            line_checkpoints: scan.line_checkpoints,
            line_checkpoint_starts: scan.line_checkpoint_starts,
        };
        let pieces = normalized_file_pieces(range_start, range_end, &scan.crlf_offsets);
        let piece_starts = piece_starts(&pieces);
        let logical_len = range_end
            .saturating_sub(range_start)
            .saturating_sub(scan.crlf_offsets.len());
        Self {
            original: OriginalBacking::from_file(file, snapshot, original_metadata),
            add: String::new(),
            pieces,
            index: LineIndex {
                line_starts: local_line_starts,
                total_bytes: logical_len,
            },
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
            piece_starts,
            undo_stack: crate::buffer::undo::UndoStack::new(),
            recording: true,
        }
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

fn normalized_file_pieces(range_start: usize, range_end: usize, crlf: &[usize]) -> Vec<Piece> {
    let mut pieces = Vec::with_capacity(crlf.len().saturating_add(1));
    let mut start = range_start;
    for &carriage_return in crlf {
        if carriage_return > start {
            pieces.push(Piece {
                source: Source::Original,
                start,
                len: carriage_return - start,
            });
        }
        start = carriage_return.saturating_add(1);
    }
    if start < range_end || pieces.is_empty() {
        pieces.push(Piece {
            source: Source::Original,
            start,
            len: range_end.saturating_sub(start),
        });
    }
    pieces
}

fn piece_starts(pieces: &[Piece]) -> Vec<usize> {
    let mut offset = 0usize;
    pieces
        .iter()
        .map(|piece| {
            let start = offset;
            offset = offset.saturating_add(piece.len);
            start
        })
        .collect()
}
