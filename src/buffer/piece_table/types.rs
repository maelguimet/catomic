//! Core types for PieceTable.
//!
//! Source, Piece, OriginalBacking, LineIndex (data), PieceTable struct definition.
//!
//! Purpose: own the storage model and construction.
//! Owns: original backing/add buffer, pieces list, line index, cursor state + byte offset cache, undo_stack + recording flag.
//! Must not: perform heavy UI or project work.
//! Invariants:
//! - Pieces are non-overlapping, cover the logical document, byte ranges respect char boundaries.
//! - OriginalBacking slices must preserve the same UTF-8 byte-boundary contract as Piece ranges.
//! - LineIndex reflects the logical newlines in the piece concatenation.
//! - cursor_byte_offset always matches the byte position of (cursor.row, cursor.col).
//!

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use std::{io, io::Write};

use crate::buffer::line_index::LineIndex;
use crate::buffer::Cursor;

#[cfg(test)]
pub(crate) use super::file_original::FileReadOperationTestPoint;
use super::file_original::{
    FileMetadataSnapshot, FileOriginal, FileOriginalMetadata, FileOriginalReadOperation,
};
use super::piece_tree::PieceTree;
use super::scalar_index::ScalarIndex;

/// Source buffer for a piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Original,
    Add,
}

/// A contiguous byte range in one of the sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub(crate) source: Source,
    /// Logical byte offset into the source. File originals map these normalized
    /// coordinates back to descriptor bytes through their CRLF elision table.
    pub(crate) start: usize,
    /// Byte length.
    pub(crate) len: usize,
    /// Cached scalar length for owned/add sources. File-backed pieces retain
    /// `None` and use their descriptor metadata for partial logical lines.
    pub(crate) char_len: Option<usize>,
}

/// Original file/input storage behind Piece ranges.
#[derive(Clone, Debug)]
pub(crate) enum OriginalBacking {
    Owned { text: String, scalars: ScalarIndex },
    File(Arc<FileOriginal>),
}

pub(crate) enum OriginalReadOperation<'a> {
    Owned {
        text: &'a str,
        scalars: &'a ScalarIndex,
    },
    File(FileOriginalReadOperation<'a>),
}

impl OriginalBacking {
    pub(crate) fn empty() -> Self {
        Self::Owned {
            text: String::new(),
            scalars: ScalarIndex::for_immutable_text(""),
        }
    }

    pub(crate) fn from_owned(text: String) -> Self {
        let scalars = ScalarIndex::for_immutable_text(&text);
        Self::Owned { text, scalars }
    }

    pub(crate) fn owned_scalar_len(&self) -> Option<usize> {
        match self {
            Self::Owned { scalars, .. } => Some(scalars.scalar_len()),
            Self::File(_) => None,
        }
    }

    pub(crate) fn from_file(
        file: std::fs::File,
        snapshot: FileMetadataSnapshot,
        metadata: Arc<FileOriginalMetadata>,
    ) -> Self {
        Self::File(Arc::new(FileOriginal::new(file, snapshot, metadata)))
    }

    /// Read a prefix that ends on a scalar boundary. The returned text may use
    /// up to three bytes beyond `max_bytes` to avoid splitting a UTF-8 scalar.
    pub(crate) fn search_text_segment(
        &self,
        range: Range<usize>,
        max_bytes: usize,
    ) -> io::Result<Cow<'_, str>> {
        match self {
            Self::Owned { text, .. } => {
                let mut end = range.start + range.len().min(max_bytes);
                while end < range.end && !text.is_char_boundary(end) {
                    end += 1;
                }
                Ok(Cow::Borrowed(&text[range.start..end]))
            }
            Self::File(file) => file.search_text_segment(range, max_bytes).map(Cow::Owned),
        }
    }

    /// Borrow owned source text without bypassing file-backed normalization.
    ///
    /// File-backed ranges use descriptor-aware reads because their logical
    /// coordinates may elide carriage returns from CRLF input.
    pub(crate) fn borrowed_slice(&self, range: Range<usize>) -> Option<&str> {
        match self {
            Self::Owned { text, .. } => Some(&text[range]),
            Self::File(_) => None,
        }
    }

    pub(crate) fn write_slice(&self, range: Range<usize>, out: &mut dyn Write) -> io::Result<()> {
        match self {
            Self::Owned { text, .. } => out.write_all(text[range].as_bytes()),
            Self::File(file) => file.write_range(range, out),
        }
    }

    pub(crate) fn for_each_newline(&self, range: Range<usize>, mut f: impl FnMut(usize)) {
        match self {
            Self::Owned { text, .. } => {
                for (i, _) in text[range.clone()].match_indices('\n') {
                    f(range.start + i);
                }
            }
            Self::File(file) => file.for_each_newline(range, f),
        }
    }

    pub(crate) fn with_read_operation<T>(
        &self,
        read: impl FnOnce(&OriginalReadOperation<'_>) -> io::Result<T>,
    ) -> io::Result<T> {
        match self {
            Self::Owned { text, scalars } => read(&OriginalReadOperation::Owned { text, scalars }),
            Self::File(file) => {
                file.with_read_operation(|operation| read(&OriginalReadOperation::File(*operation)))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn file_read_bytes(&self) -> usize {
        match self {
            Self::Owned { .. } => 0,
            Self::File(file) => file.read_bytes(),
        }
    }

    #[cfg(test)]
    pub(crate) fn metadata_check_count(&self) -> usize {
        match self {
            Self::Owned { .. } => 0,
            Self::File(file) => file.metadata_check_count(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_file_read_operation_test_hook(
        &self,
        point: FileReadOperationTestPoint,
        action: impl FnOnce() + Send + 'static,
    ) {
        if let Self::File(file) = self {
            file.set_read_operation_test_hook(point, action);
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        match self {
            Self::Owned { text, scalars } => text.capacity() + scalars.retained_bytes(),
            Self::File(file) => {
                std::mem::size_of::<FileOriginal>() + file.retained_metadata_bytes()
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_metadata_bytes(&self) -> usize {
        match self {
            Self::Owned { .. } => 0,
            Self::File(file) => file.retained_metadata_bytes(),
        }
    }

    #[cfg(test)]
    pub(crate) fn file_metadata_bytes(
        &self,
    ) -> Option<super::file_original::FileOriginalMetadataBytes> {
        match self {
            Self::Owned { .. } => None,
            Self::File(file) => Some(file.metadata_bytes()),
        }
    }

    #[cfg(test)]
    pub(crate) fn take_scalar_visited_bytes(&self) -> usize {
        match self {
            Self::Owned { scalars, .. } => scalars.take_visited_bytes(),
            Self::File(_) => 0,
        }
    }
}

impl OriginalReadOperation<'_> {
    pub(crate) fn try_push_slice(&self, range: Range<usize>, out: &mut String) -> io::Result<()> {
        match self {
            Self::Owned { text, .. } => {
                out.push_str(&text[range]);
                Ok(())
            }
            Self::File(file) => file.push_range(range, out),
        }
    }

    pub(crate) fn try_char_count(&self, range: Range<usize>) -> io::Result<usize> {
        match self {
            Self::Owned { text, scalars } => Ok(scalars.scalar_count(text, range)),
            Self::File(file) => file.char_count(range),
        }
    }

    pub(crate) fn try_byte_offset_at_char(
        &self,
        range: Range<usize>,
        col: usize,
    ) -> io::Result<usize> {
        match self {
            Self::Owned { text, scalars } => Ok(scalars.byte_at_scalar_in(text, range, col)),
            Self::File(file) => file.byte_offset_at_char(range, col),
        }
    }
}

// LineIndex lives in crate::buffer::line_index (single definition, no duplicate).
// PT stores and uses it.

/// PieceTable with original + add + pieces + index + cached cursor offset.
#[derive(Clone, Debug)]
pub struct PieceTable {
    pub(crate) original: OriginalBacking,
    pub(crate) add: String,
    pub(crate) add_scalars: ScalarIndex,
    pub(crate) pieces: PieceTree,
    pub(crate) index: LineIndex,
    pub(crate) cursor: Cursor,
    /// Cached global logical byte offset for the cursor.
    /// Avoids full rebuild on every edit for offset calculation.
    pub(crate) cursor_byte_offset: usize,
    /// Undo/redo history (piece deltas only; no full snapshots).
    pub(crate) undo_stack: crate::buffer::undo::UndoStack,
    /// If false, structural edits do not record transactions (suppress during apply).
    pub(crate) recording: bool,
    #[cfg(test)]
    pub(crate) last_piece_mutation: PieceMutationMetrics,
    #[cfg(test)]
    pub(crate) compaction_descriptor_scans: std::cell::Cell<usize>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PieceMutationMetrics {
    pub(crate) pieces_touched: usize,
    pub(crate) pieces_allocated: usize,
}
