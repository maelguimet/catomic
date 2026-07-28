//! Purpose: provide immutable descriptor-backed original bytes for PieceTable.
//! Owns: descriptor snapshot checks, positioned range reads, original-line
//!   scalar metadata, compact CRLF source mapping, checkpoint-assisted cursor
//!   mapping, and streamed ranges.
//! Must not: own logical pieces, edits, App policy, rendering, or external services.
//! Invariants: Piece ranges use normalized logical coordinates; mapped source
//!   ranges are UTF-8 boundaries; CR bytes belonging to CRLF never enter logical
//!   text; line metadata describes the scanned descriptor; metadata drift fails
//!   fallible reads and writes closed.

use std::fs::File;
use std::io::{self, Write};
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::sync::Arc;

use crate::buffer::large_file::LineCheckpoint;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileReadOperationTestPoint {
    BeforeInitialValidation,
    AfterRangeRead,
    BeforeFinalValidation,
}

#[cfg(test)]
struct FileReadOperationTestHook {
    point: FileReadOperationTestPoint,
    action: Option<Box<dyn FnOnce() + Send>>,
}

#[cfg(test)]
impl std::fmt::Debug for FileReadOperationTestHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileReadOperationTestHook")
            .field("point", &self.point)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileMetadataSnapshot {
    pub(crate) len: u64,
    mtime: Option<std::time::SystemTime>,
}

impl FileMetadataSnapshot {
    pub(crate) fn capture(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            len: metadata.len(),
            mtime: metadata.modified().ok(),
        })
    }
}

/// Page-local offsets use `u32` when possible. A page that does not fit keeps
/// exact `usize` offsets rather than imposing an artificial 4-GiB page limit.
#[derive(Debug)]
pub(crate) enum CompactOffsets {
    Relative32 { base: usize, values: Vec<u32> },
    Wide(Vec<usize>),
}

impl CompactOffsets {
    pub(crate) fn from_absolute(values: Vec<usize>) -> Self {
        let base = values.first().copied().unwrap_or(0);
        if values.iter().all(|value| {
            value
                .checked_sub(base)
                .is_some_and(|offset| u32::try_from(offset).is_ok())
        }) {
            Self::Relative32 {
                base,
                values: values
                    .into_iter()
                    .map(|value| u32::try_from(value - base).expect("checked compact offset"))
                    .collect(),
            }
        } else {
            Self::Wide(values)
        }
    }

    pub(crate) fn from_values(values: Vec<usize>) -> Self {
        if values.iter().all(|value| u32::try_from(*value).is_ok()) {
            Self::Relative32 {
                base: 0,
                values: values
                    .into_iter()
                    .map(|value| u32::try_from(value).expect("checked compact value"))
                    .collect(),
            }
        } else {
            Self::Wide(values)
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Relative32 { values, .. } => values.len(),
            Self::Wide(values) => values.len(),
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<usize> {
        match self {
            Self::Relative32 { base, values } => {
                values.get(index).map(|value| base + *value as usize)
            }
            Self::Wide(values) => values.get(index).copied(),
        }
    }

    pub(crate) fn partition_point(&self, mut predicate: impl FnMut(usize) -> bool) -> usize {
        let mut left = 0usize;
        let mut right = self.len();
        while left < right {
            let middle = (left + right) / 2;
            if predicate(self.get(middle).expect("compact offset index in bounds")) {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        match self {
            Self::Relative32 { values, .. } => values.capacity() * std::mem::size_of::<u32>(),
            Self::Wide(values) => values.capacity() * std::mem::size_of::<usize>(),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_wide(&self) -> bool {
        matches!(self, Self::Wide(_))
    }
}

#[derive(Debug)]
pub(crate) struct FileOriginalMetadata {
    pub(crate) range_start: usize,
    pub(crate) range_end: usize,
    pub(crate) logical_len: usize,
    /// Canonical absolute descriptor offsets of logical line starts.
    pub(crate) line_starts: CompactOffsets,
    /// Absolute descriptor offsets of CR bytes elided from CRLF.
    pub(crate) crlf_offsets: CompactOffsets,
    /// Only non-ASCII rows retain scalar counts and checkpoint boundaries.
    non_ascii_rows: CompactOffsets,
    non_ascii_char_counts: CompactOffsets,
    non_ascii_checkpoint_starts: CompactOffsets,
    line_checkpoints: Vec<LineCheckpoint>,
}

impl FileOriginalMetadata {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_scan(
        range_start: usize,
        range_end: usize,
        logical_len: usize,
        line_starts: Vec<usize>,
        crlf_offsets: Vec<usize>,
        line_char_counts: Vec<usize>,
        line_is_ascii: Vec<bool>,
        line_checkpoints: Vec<LineCheckpoint>,
        line_checkpoint_starts: Vec<usize>,
    ) -> Self {
        let mut non_ascii_rows = Vec::new();
        let mut non_ascii_char_counts = Vec::new();
        let mut compact_checkpoints = Vec::new();
        let mut compact_checkpoint_starts = Vec::new();
        for (row, is_ascii) in line_is_ascii.into_iter().enumerate() {
            if is_ascii {
                continue;
            }
            non_ascii_rows.push(row);
            non_ascii_char_counts.push(line_char_counts.get(row).copied().unwrap_or(0));
            if compact_checkpoint_starts.is_empty() {
                compact_checkpoint_starts.push(0);
            }
            let start = line_checkpoint_starts.get(row).copied().unwrap_or(0);
            let end = line_checkpoint_starts
                .get(row + 1)
                .copied()
                .unwrap_or(line_checkpoints.len());
            compact_checkpoints.extend_from_slice(&line_checkpoints[start..end]);
            compact_checkpoint_starts.push(compact_checkpoints.len());
        }
        Self {
            range_start,
            range_end,
            logical_len,
            line_starts: CompactOffsets::from_absolute(line_starts),
            crlf_offsets: CompactOffsets::from_absolute(crlf_offsets),
            non_ascii_rows: CompactOffsets::from_values(non_ascii_rows),
            non_ascii_char_counts: CompactOffsets::from_values(non_ascii_char_counts),
            non_ascii_checkpoint_starts: CompactOffsets::from_values(compact_checkpoint_starts),
            line_checkpoints: compact_checkpoints,
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    fn raw_line_start(&self, row: usize) -> usize {
        self.line_starts
            .get(row.min(self.line_count().saturating_sub(1)))
            .unwrap_or(self.range_start)
    }

    fn raw_line_end(&self, row: usize) -> usize {
        self.line_starts
            .get(row.saturating_add(1))
            .map(|start| start.saturating_sub(1))
            .unwrap_or(self.range_end)
    }

    pub(crate) fn logical_line_start(&self, row: usize) -> usize {
        let raw = self.raw_line_start(row);
        let removed = self.crlf_offsets.partition_point(|offset| offset < raw);
        raw.saturating_sub(self.range_start).saturating_sub(removed)
    }

    pub(crate) fn logical_line_end(&self, row: usize) -> usize {
        if row + 1 < self.line_count() {
            self.logical_line_start(row + 1).saturating_sub(1)
        } else {
            self.logical_len
        }
    }

    pub(crate) fn row_for_logical_byte(&self, byte: usize) -> usize {
        let mut left = 0usize;
        let mut right = self.line_count();
        while left < right {
            let middle = (left + right) / 2;
            if self.logical_line_start(middle) <= byte {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left.saturating_sub(1)
            .min(self.line_count().saturating_sub(1))
    }

    /// Build normalized line spans in one merge pass over line starts and CRLF
    /// elisions. First-edit materialization is linear rather than one binary
    /// search per row.
    pub(crate) fn logical_line_spans(&self) -> Vec<usize> {
        let mut spans = Vec::with_capacity(self.line_count());
        let mut previous = 0usize;
        let mut crlf_index = 0usize;
        for row in 0..self.line_count() {
            let raw = self.raw_line_start(row);
            while self
                .crlf_offsets
                .get(crlf_index)
                .is_some_and(|offset| offset < raw)
            {
                crlf_index += 1;
            }
            let logical = raw
                .saturating_sub(self.range_start)
                .saturating_sub(crlf_index);
            if row > 0 {
                spans.push(logical.saturating_sub(previous));
            }
            previous = logical;
        }
        spans.push(self.logical_len.saturating_sub(previous));
        spans
    }

    fn non_ascii_index(&self, row: usize) -> Option<usize> {
        let index = self
            .non_ascii_rows
            .partition_point(|candidate| candidate < row);
        (self.non_ascii_rows.get(index) == Some(row)).then_some(index)
    }

    fn is_crlf_before(&self, offset: usize) -> bool {
        let index = self
            .crlf_offsets
            .partition_point(|candidate| candidate < offset);
        self.crlf_offsets.get(index) == Some(offset)
    }

    fn line_char_count(&self, row: usize) -> usize {
        if let Some(index) = self.non_ascii_index(row) {
            return self.non_ascii_char_counts.get(index).unwrap_or(0);
        }
        let start = self.raw_line_start(row);
        let end = self.raw_line_end(row);
        end.saturating_sub(start)
            .saturating_sub(usize::from(end > start && self.is_crlf_before(end - 1)))
    }

    fn line_checkpoints(&self, row: usize) -> &[LineCheckpoint] {
        let Some(index) = self.non_ascii_index(row) else {
            return &[];
        };
        let start = self.non_ascii_checkpoint_starts.get(index).unwrap_or(0);
        let end = self
            .non_ascii_checkpoint_starts
            .get(index + 1)
            .unwrap_or(self.line_checkpoints.len());
        &self.line_checkpoints[start..end]
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> FileOriginalMetadataBytes {
        FileOriginalMetadataBytes {
            line_starts: self.line_starts.retained_bytes(),
            materialized_line_index: 0,
            crlf_offsets: self.crlf_offsets.retained_bytes(),
            non_ascii_rows: self.non_ascii_rows.retained_bytes(),
            non_ascii_char_counts: self.non_ascii_char_counts.retained_bytes(),
            non_ascii_checkpoint_starts: self.non_ascii_checkpoint_starts.retained_bytes(),
            checkpoints: self.line_checkpoints.capacity() * std::mem::size_of::<LineCheckpoint>(),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileOriginalMetadataBytes {
    pub(crate) line_starts: usize,
    pub(crate) materialized_line_index: usize,
    pub(crate) crlf_offsets: usize,
    pub(crate) non_ascii_rows: usize,
    pub(crate) non_ascii_char_counts: usize,
    pub(crate) non_ascii_checkpoint_starts: usize,
    pub(crate) checkpoints: usize,
}

#[cfg(test)]
impl FileOriginalMetadataBytes {
    pub(crate) fn total(&self) -> usize {
        self.line_starts
            + self.materialized_line_index
            + self.crlf_offsets
            + self.non_ascii_rows
            + self.non_ascii_char_counts
            + self.non_ascii_checkpoint_starts
            + self.checkpoints
    }
}

#[derive(Debug)]
pub(crate) struct FileOriginal {
    file: File,
    snapshot: FileMetadataSnapshot,
    metadata: Arc<FileOriginalMetadata>,
    #[cfg(test)]
    read_bytes: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    metadata_check_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    read_operation_test_hook: std::sync::Mutex<Option<FileReadOperationTestHook>>,
}

#[derive(Clone, Copy)]
pub(crate) struct FileOriginalReadOperation<'a> {
    original: &'a FileOriginal,
    expected_snapshot: FileMetadataSnapshot,
}

impl FileOriginal {
    pub(crate) fn new(
        file: File,
        snapshot: FileMetadataSnapshot,
        metadata: Arc<FileOriginalMetadata>,
    ) -> Self {
        Self {
            file,
            snapshot,
            metadata,
            #[cfg(test)]
            read_bytes: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            metadata_check_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            read_operation_test_hook: std::sync::Mutex::new(None),
        }
    }

    fn ensure_snapshot(&self, expected: FileMetadataSnapshot) -> io::Result<()> {
        #[cfg(test)]
        self.metadata_check_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if FileMetadataSnapshot::capture(&self.file)? == expected {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-backed original changed while open",
            ))
        }
    }

    fn ensure_unchanged(&self) -> io::Result<()> {
        self.ensure_snapshot(self.snapshot)
    }

    pub(crate) fn with_read_operation<T>(
        &self,
        read: impl FnOnce(&FileOriginalReadOperation<'_>) -> io::Result<T>,
    ) -> io::Result<T> {
        #[cfg(test)]
        self.run_read_operation_test_hook(FileReadOperationTestPoint::BeforeInitialValidation);
        let expected_snapshot = self.snapshot;
        self.ensure_snapshot(expected_snapshot)?;
        let operation = FileOriginalReadOperation {
            original: self,
            expected_snapshot,
        };
        let result = read(&operation);
        #[cfg(test)]
        self.run_read_operation_test_hook(FileReadOperationTestPoint::BeforeFinalValidation);
        self.ensure_snapshot(operation.expected_snapshot)?;
        result
    }

    fn read_range_unchecked(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0u8; range.len()];
        let mut filled = 0usize;
        while filled < bytes.len() {
            let read = self
                .file
                .read_at(&mut bytes[filled..], (range.start + filled) as u64)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read from file-backed original",
                ));
            }
            #[cfg(test)]
            self.read_bytes
                .fetch_add(read, std::sync::atomic::Ordering::Relaxed);
            filled += read;
        }
        #[cfg(test)]
        self.run_read_operation_test_hook(FileReadOperationTestPoint::AfterRangeRead);
        Ok(bytes)
    }

    /// Read a scalar-aligned prefix without requiring line metadata. Search can
    /// traverse pieces that span original newlines, so it cannot use the
    /// line-oriented cursor helpers below.
    pub(crate) fn search_text_segment(
        &self,
        logical_range: Range<usize>,
        max_bytes: usize,
    ) -> io::Result<String> {
        if logical_range.is_empty() || max_bytes == 0 {
            return Ok(String::new());
        }
        self.ensure_unchanged()?;
        let logical_read_end = logical_range
            .start
            .saturating_add(max_bytes.saturating_add(3))
            .min(logical_range.end);
        let source_range = self.source_range(logical_range.start..logical_read_end)?;
        let bytes = self.read_range_unchecked(source_range.clone())?;
        let mut normalized = Vec::with_capacity(bytes.len());
        let mut source_start = source_range.start;
        self.for_each_crlf_offset(source_range.clone(), |carriage_return| {
            normalized.extend_from_slice(
                &bytes[source_start - source_range.start..carriage_return - source_range.start],
            );
            source_start = carriage_return + 1;
            Ok(())
        })?;
        normalized.extend_from_slice(&bytes[source_start - source_range.start..]);
        let text = utf8_valid_prefix(&normalized)?;
        let mut end = 0;
        for (start, ch) in text.char_indices() {
            end = start + ch.len_utf8();
            if end >= max_bytes {
                break;
            }
        }
        self.ensure_unchanged()?;
        Ok(text[..end].to_owned())
    }

    pub(crate) fn write_range(
        &self,
        logical_range: Range<usize>,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let range = self.source_range(logical_range)?;
        self.ensure_unchanged()?;
        let mut offset = range.start;
        while offset < range.end {
            let end = offset.saturating_add(64 * 1024).min(range.end);
            let bytes = self.read_range_unchecked(offset..end)?;
            let first_cr = self
                .metadata
                .crlf_offsets
                .partition_point(|carriage_return| carriage_return < offset);
            if self
                .metadata
                .crlf_offsets
                .get(first_cr)
                .is_none_or(|carriage_return| carriage_return >= end)
            {
                out.write_all(&bytes)?;
            } else {
                let mut normalized = Vec::with_capacity(bytes.len());
                let mut source_start = offset;
                for index in first_cr..self.metadata.crlf_offsets.len() {
                    let carriage_return = self
                        .metadata
                        .crlf_offsets
                        .get(index)
                        .expect("compact CRLF index in bounds");
                    if carriage_return >= end {
                        break;
                    }
                    normalized
                        .extend_from_slice(&bytes[source_start - offset..carriage_return - offset]);
                    source_start = carriage_return + 1;
                }
                normalized.extend_from_slice(&bytes[source_start - offset..]);
                out.write_all(&normalized)?;
            }
            offset = end;
        }
        self.ensure_unchanged()
    }

    pub(crate) fn for_each_newline(&self, range: Range<usize>, mut f: impl FnMut(usize)) {
        let mut row = self.metadata.row_for_logical_byte(range.start);
        if self.metadata.logical_line_start(row) <= range.start {
            row = row.saturating_add(1);
        }
        let first_raw = self.metadata.raw_line_start(row);
        let mut crlf_index = self
            .metadata
            .crlf_offsets
            .partition_point(|offset| offset < first_raw);
        while row < self.metadata.line_count() {
            let raw = self.metadata.raw_line_start(row);
            while self
                .metadata
                .crlf_offsets
                .get(crlf_index)
                .is_some_and(|offset| offset < raw)
            {
                crlf_index += 1;
            }
            let newline = raw
                .saturating_sub(self.metadata.range_start)
                .saturating_sub(crlf_index)
                .saturating_sub(1);
            if newline >= range.end {
                break;
            }
            if newline >= range.start {
                f(newline);
            }
            row += 1;
        }
    }

    fn range_columns(&self, range: &Range<usize>) -> io::Result<(usize, usize, usize)> {
        if range.start > range.end || range.end > self.metadata.logical_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file-original logical range is invalid",
            ));
        }
        let row = self.row_for_byte(range.start);
        if range.end > self.line_end(row) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "piece query crosses an original newline",
            ));
        }
        let start_col = self.line_col_at_byte(row, range.start)?;
        let end_col = self.line_col_at_byte(row, range.end)?;
        Ok((row, start_col, end_col))
    }

    fn row_for_byte(&self, byte: usize) -> usize {
        self.metadata.row_for_logical_byte(byte)
    }

    fn line_start(&self, row: usize) -> usize {
        self.metadata.logical_line_start(row)
    }

    fn line_end(&self, row: usize) -> usize {
        self.metadata.logical_line_end(row)
    }

    fn line_checkpoints(&self, row: usize) -> &[LineCheckpoint] {
        self.metadata.line_checkpoints(row)
    }

    fn line_col_at_byte(&self, row: usize, byte: usize) -> io::Result<usize> {
        if self.metadata.non_ascii_index(row).is_none() {
            return Ok(byte.saturating_sub(self.line_start(row)));
        }
        let source_byte = self.logical_to_source_end(byte);
        let checkpoints = self.line_checkpoints(row);
        let idx = checkpoints.partition_point(|checkpoint| checkpoint.byte_offset <= source_byte);
        let checkpoint = idx.checked_sub(1).map(|i| checkpoints[i]);
        let start = checkpoint.map_or_else(
            || self.logical_to_source_start(self.line_start(row)),
            |item| item.byte_offset,
        );
        let col = checkpoint.map_or(0, |item| item.col);
        let bytes = self.read_range_unchecked(start..source_byte)?;
        Ok(col + as_utf8(&bytes)?.chars().count())
    }

    fn byte_offset_at_line_col(&self, row: usize, col: usize) -> io::Result<usize> {
        let line_chars = self.metadata.line_char_count(row);
        let col = col.min(line_chars);
        if self.metadata.non_ascii_index(row).is_none() {
            return Ok(self.line_start(row) + col);
        }
        let checkpoints = self.line_checkpoints(row);
        let idx = checkpoints.partition_point(|checkpoint| checkpoint.col <= col);
        let checkpoint = idx.checked_sub(1).map(|i| checkpoints[i]);
        let start = checkpoint.map_or_else(
            || self.logical_to_source_start(self.line_start(row)),
            |item| item.byte_offset,
        );
        let start_col = checkpoint.map_or(0, |item| item.col);
        let remaining = col - start_col;
        if remaining == 0 {
            return Ok(self.source_to_logical(start));
        }
        let read_end = start
            .saturating_add(remaining.saturating_mul(4))
            .min(self.logical_to_source_end(self.line_end(row)));
        let bytes = self.read_range_unchecked(start..read_end)?;
        let text = utf8_valid_prefix(&bytes)?;
        let relative = text
            .char_indices()
            .nth(remaining)
            .map_or(text.len(), |(offset, _)| offset);
        Ok(self.source_to_logical(start + relative))
    }

    fn source_range(&self, logical: Range<usize>) -> io::Result<Range<usize>> {
        if logical.start > logical.end || logical.end > self.metadata.logical_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file-original logical range is invalid",
            ));
        }
        if logical.is_empty() {
            let source = self.logical_to_source_start(logical.start);
            return Ok(source..source);
        }
        Ok(self.logical_to_source_start(logical.start)..self.logical_to_source_end(logical.end))
    }

    /// Map a logical range start after any CR elided at that position, so a
    /// slice beginning on a normalized newline begins on its LF byte.
    fn logical_to_source_start(&self, logical: usize) -> usize {
        let removed = self.crlf_count_at_logical(logical, true);
        self.metadata
            .range_start
            .saturating_add(logical)
            .saturating_add(removed)
            .min(self.metadata.range_end)
    }

    /// Map a logical range end before any CR elided at that position, so a
    /// slice ending before a normalized newline excludes the CR as well.
    fn logical_to_source_end(&self, logical: usize) -> usize {
        let removed = self.crlf_count_at_logical(logical, false);
        self.metadata
            .range_start
            .saturating_add(logical)
            .saturating_add(removed)
            .min(self.metadata.range_end)
    }

    fn source_to_logical(&self, source: usize) -> usize {
        let removed = self
            .metadata
            .crlf_offsets
            .partition_point(|offset| offset < source);
        source
            .saturating_sub(self.metadata.range_start)
            .saturating_sub(removed)
    }

    fn crlf_count_at_logical(&self, logical: usize, include_equal: bool) -> usize {
        let mut left = 0usize;
        let mut right = self.metadata.crlf_offsets.len();
        while left < right {
            let middle = (left + right) / 2;
            let elision = self
                .metadata
                .crlf_offsets
                .get(middle)
                .expect("compact CRLF index in bounds")
                .saturating_sub(self.metadata.range_start)
                .saturating_sub(middle);
            if elision < logical || (include_equal && elision == logical) {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }

    fn for_each_crlf_offset(
        &self,
        range: Range<usize>,
        mut visit: impl FnMut(usize) -> io::Result<()>,
    ) -> io::Result<()> {
        let start = self
            .metadata
            .crlf_offsets
            .partition_point(|offset| offset < range.start);
        for index in start..self.metadata.crlf_offsets.len() {
            let offset = self
                .metadata
                .crlf_offsets
                .get(index)
                .expect("compact CRLF index in bounds");
            if offset >= range.end {
                break;
            }
            visit(offset)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn read_bytes(&self) -> usize {
        self.read_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn metadata_check_count(&self) -> usize {
        self.metadata_check_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn set_read_operation_test_hook(
        &self,
        point: FileReadOperationTestPoint,
        action: impl FnOnce() + Send + 'static,
    ) {
        *self.read_operation_test_hook.lock().unwrap() = Some(FileReadOperationTestHook {
            point,
            action: Some(Box::new(action)),
        });
    }

    #[cfg(test)]
    fn run_read_operation_test_hook(&self, point: FileReadOperationTestPoint) {
        let action = {
            let mut slot = self.read_operation_test_hook.lock().unwrap();
            if slot.as_ref().is_some_and(|hook| hook.point == point) {
                slot.take().and_then(|mut hook| hook.action.take())
            } else {
                None
            }
        };
        if let Some(action) = action {
            action();
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_metadata_bytes(&self) -> usize {
        self.metadata.retained_bytes().total()
    }

    #[cfg(test)]
    pub(crate) fn metadata_bytes(&self) -> FileOriginalMetadataBytes {
        self.metadata.retained_bytes()
    }
}

impl FileOriginalReadOperation<'_> {
    pub(crate) fn push_range(
        &self,
        logical_range: Range<usize>,
        out: &mut String,
    ) -> io::Result<()> {
        let source_range = self.original.source_range(logical_range)?;
        let bytes = self.read_range(source_range.clone())?;
        let mut source_start = source_range.start;
        self.original
            .for_each_crlf_offset(source_range.clone(), |carriage_return| {
                let relative_start = source_start - source_range.start;
                let relative_end = carriage_return - source_range.start;
                if relative_start < relative_end {
                    out.push_str(as_utf8(&bytes[relative_start..relative_end])?);
                }
                source_start = carriage_return + 1;
                Ok(())
            })?;
        let relative_start = source_start - source_range.start;
        if relative_start < bytes.len() {
            out.push_str(as_utf8(&bytes[relative_start..])?);
        }
        Ok(())
    }

    pub(crate) fn char_count(&self, range: Range<usize>) -> io::Result<usize> {
        let (_row, start_col, end_col) = self.original.range_columns(&range)?;
        Ok(end_col - start_col)
    }

    pub(crate) fn byte_offset_at_char(&self, range: Range<usize>, col: usize) -> io::Result<usize> {
        let (row, start_col, end_col) = self.original.range_columns(&range)?;
        let target_col = start_col.saturating_add(col).min(end_col);
        self.original.byte_offset_at_line_col(row, target_col)
    }

    fn read_range(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        self.original.read_range_unchecked(range)
    }
}

fn as_utf8(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn utf8_valid_prefix(bytes: &[u8]) -> io::Result<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.error_len().is_none() => as_utf8(&bytes[..error.valid_up_to()]),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::{CompactOffsets, FileOriginalMetadata};

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn compact_offsets_keep_small_values_in_u32_storage() {
        let offsets = CompactOffsets::from_absolute(vec![
            4 * 1024 * 1024 * 1024,
            4 * 1024 * 1024 * 1024 + 32,
        ]);

        assert!(!offsets.is_wide());
        assert_eq!(offsets.get(1), Some(4 * 1024 * 1024 * 1024 + 32));
        assert_eq!(offsets.retained_bytes(), 2 * std::mem::size_of::<u32>());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn wide_offsets_preserve_line_lookup_and_crlf_normalization() {
        let second_start = 17 + u32::MAX as usize + 9;
        let crlf = second_start - 2;
        let logical_len = second_start - 17 - 1 + 3;
        let metadata = FileOriginalMetadata::from_scan(
            17,
            second_start + 3,
            logical_len,
            vec![17, second_start],
            vec![crlf],
            vec![u32::MAX as usize + 6, 3],
            vec![true, true],
            Vec::new(),
            vec![0, 0],
        );

        assert!(metadata.line_starts.is_wide());
        assert_eq!(metadata.logical_line_start(1), second_start - 17 - 1);
        assert_eq!(metadata.row_for_logical_byte(second_start - 17 - 2), 0);
        assert_eq!(metadata.row_for_logical_byte(second_start - 17 - 1), 1);
        assert_eq!(
            metadata.logical_line_spans(),
            vec![second_start - 17 - 1, 3]
        );
    }
}
