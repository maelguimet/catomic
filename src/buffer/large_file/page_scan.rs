//! Purpose: scan one configured line page from a Huge-file descriptor.
//! Owns: bounded page boundaries, chunk-boundary UTF-8 validation, and page metadata.
//! Must not: reopen paths, retain descriptors, render, edit, or choose App policy.
//! Invariants: a complete non-final page ends after its configured newline count;
//!   page metadata uses absolute descriptor offsets and retains no file content.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

use memchr::{memchr, memrchr};

use super::scan::{LineScan, LineScanState};
use super::SCAN_CHUNK_BYTES;

pub(crate) struct PageScan {
    pub(crate) lines: LineScan,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) next_page_start: Option<usize>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PageScanPerfStats {
    pub(crate) logical_bytes_examined: usize,
    pub(crate) descriptor_read_calls: usize,
    pub(crate) descriptor_read_bytes: usize,
    pub(crate) newline_count: usize,
}

#[cfg(test)]
pub(crate) struct PreviousPageScan {
    pub(crate) start_byte: usize,
    pub(crate) perf: PageScanPerfStats,
}

pub(crate) fn scan_utf8_page(
    file: &File,
    start_byte: usize,
    page_lines: usize,
) -> io::Result<PageScan> {
    scan_utf8_page_with_reader(start_byte, page_lines, |out, offset| {
        file.read_at(out, offset as u64)
    })
}

fn scan_utf8_page_with_reader(
    start_byte: usize,
    page_lines: usize,
    mut read_chunk: impl FnMut(&mut [u8], usize) -> io::Result<usize>,
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
        let n = read_chunk(&mut chunk, state.offset)?;
        if n == 0 {
            break;
        }
        if state.scan_chunk(&chunk[..n])? {
            state.finish_complete_page()?;
            let next_page_start = state.offset;
            return Ok(state.into_scan(Some(next_page_start)));
        }
    }
    state.finish_final_page()?;
    Ok(state.into_scan(None))
}

#[cfg(test)]
pub(crate) fn scan_utf8_page_for_perf(
    file: &File,
    start_byte: usize,
    page_lines: usize,
) -> io::Result<(PageScan, PageScanPerfStats)> {
    let mut perf = PageScanPerfStats::default();
    let scan = scan_utf8_page_with_reader(start_byte, page_lines, |out, offset| {
        let read = file.read_at(out, offset as u64)?;
        if read > 0 {
            perf.descriptor_read_calls += 1;
            perf.descriptor_read_bytes += read;
        }
        Ok(read)
    })?;
    perf.logical_bytes_examined = scan.end_byte - scan.start_byte;
    perf.newline_count = if scan.next_page_start.is_some() {
        page_lines
    } else {
        scan.lines.line_starts.len().saturating_sub(1)
    };
    Ok((scan, perf))
}

#[cfg(test)]
pub(crate) fn scan_utf8_page_bytes_for_perf(
    bytes: &[u8],
    start_byte: usize,
    page_lines: usize,
) -> io::Result<PageScan> {
    if page_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "page line count must be positive",
        ));
    }
    if start_byte > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "page start exceeds in-memory fixture",
        ));
    }
    let mut state = PageScanState::new(start_byte, page_lines);
    while state.offset < bytes.len() {
        let end = (state.offset + SCAN_CHUNK_BYTES).min(bytes.len());
        let chunk = &bytes[state.offset..end];
        if state.scan_chunk(chunk)? {
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
    find_previous_page(file, current_start, page_lines)
}

#[cfg(test)]
pub(crate) fn find_previous_page_start_for_perf(
    file: &File,
    current_start: usize,
    page_lines: usize,
) -> io::Result<PreviousPageScan> {
    let mut perf = PageScanPerfStats::default();
    let mut observer = PerfReversePageObserver::default();
    let start_byte = find_previous_page_with_reader(
        current_start,
        page_lines,
        |out, offset| {
            let (calls, bytes) = read_exact_at_for_perf(file, out, offset)?;
            perf.descriptor_read_calls += calls;
            perf.descriptor_read_bytes += bytes;
            Ok(())
        },
        &mut observer,
    )?;
    perf.logical_bytes_examined = observer.logical_bytes_examined;
    perf.newline_count = observer.newline_count;
    Ok(PreviousPageScan { start_byte, perf })
}

#[cfg(test)]
pub(crate) fn find_previous_page_start_bytes_for_perf(
    bytes: &[u8],
    current_start: usize,
    page_lines: usize,
) -> io::Result<usize> {
    find_previous_page_in_bytes(
        bytes,
        current_start,
        page_lines,
        &mut NoopReversePageObserver,
    )
}

#[cfg(test)]
pub(crate) fn capture_previous_page_start_bytes_for_perf(
    bytes: &[u8],
    current_start: usize,
    page_lines: usize,
) -> io::Result<PreviousPageScan> {
    let mut observer = PerfReversePageObserver::default();
    let start_byte = find_previous_page_in_bytes(bytes, current_start, page_lines, &mut observer)?;
    Ok(PreviousPageScan {
        start_byte,
        perf: PageScanPerfStats {
            logical_bytes_examined: observer.logical_bytes_examined,
            newline_count: observer.newline_count,
            ..PageScanPerfStats::default()
        },
    })
}

#[cfg(test)]
fn find_previous_page_in_bytes(
    bytes: &[u8],
    current_start: usize,
    page_lines: usize,
    observer: &mut impl ReversePageObserver,
) -> io::Result<usize> {
    if current_start > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reverse page start exceeds in-memory fixture",
        ));
    }
    let mut remaining_newlines = page_lines.saturating_add(1);
    let mut end = current_start;
    while end > 0 {
        let start = end.saturating_sub(SCAN_CHUNK_BYTES);
        let chunk = reverse_page_chunk(&bytes[start..end], start, remaining_newlines);
        observer.scanned_chunk(chunk.logical_bytes_examined, chunk.newline_count);
        if let Some(start_byte) = chunk.start_byte {
            return Ok(start_byte);
        }
        remaining_newlines = remaining_newlines.saturating_sub(chunk.newline_count);
        end = start;
    }
    Ok(0)
}

fn find_previous_page(file: &File, current_start: usize, page_lines: usize) -> io::Result<usize> {
    find_previous_page_with_reader(
        current_start,
        page_lines,
        |out, offset| read_exact_at(file, out, offset),
        &mut NoopReversePageObserver,
    )
}

fn find_previous_page_with_reader(
    current_start: usize,
    page_lines: usize,
    mut read_chunk: impl FnMut(&mut [u8], usize) -> io::Result<()>,
    observer: &mut impl ReversePageObserver,
) -> io::Result<usize> {
    let mut remaining_newlines = page_lines.saturating_add(1);
    let mut end = current_start;
    let mut chunk = vec![0u8; SCAN_CHUNK_BYTES];
    while end > 0 {
        let start = end.saturating_sub(chunk.len());
        let len = end - start;
        read_chunk(&mut chunk[..len], start)?;
        let scanned = reverse_page_chunk(&chunk[..len], start, remaining_newlines);
        observer.scanned_chunk(scanned.logical_bytes_examined, scanned.newline_count);
        if let Some(start_byte) = scanned.start_byte {
            return Ok(start_byte);
        }
        remaining_newlines = remaining_newlines.saturating_sub(scanned.newline_count);
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

#[cfg(test)]
fn read_exact_at_for_perf(
    file: &File,
    mut out: &mut [u8],
    mut offset: usize,
) -> io::Result<(usize, usize)> {
    let mut read_calls = 0usize;
    let mut read_bytes = 0usize;
    while !out.is_empty() {
        let read = file.read_at(out, offset as u64)?;
        read_calls += 1;
        read_bytes += read;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short read while locating previous file page",
            ));
        }
        offset += read;
        out = &mut out[read..];
    }
    Ok((read_calls, read_bytes))
}

struct ReversePageChunk {
    start_byte: Option<usize>,
    logical_bytes_examined: usize,
    newline_count: usize,
}

trait ReversePageObserver {
    #[inline(always)]
    fn scanned_chunk(&mut self, _logical_bytes_examined: usize, _newline_count: usize) {}
}

struct NoopReversePageObserver;

impl ReversePageObserver for NoopReversePageObserver {}

#[cfg(test)]
#[derive(Default)]
struct PerfReversePageObserver {
    logical_bytes_examined: usize,
    newline_count: usize,
}

#[cfg(test)]
impl ReversePageObserver for PerfReversePageObserver {
    fn scanned_chunk(&mut self, logical_bytes_examined: usize, newline_count: usize) {
        self.logical_bytes_examined += logical_bytes_examined;
        self.newline_count += newline_count;
    }
}

fn reverse_page_chunk(
    bytes: &[u8],
    absolute_start: usize,
    remaining_newlines: usize,
) -> ReversePageChunk {
    let mut newline_count = 0usize;
    let mut search_end = bytes.len();
    while let Some(index) = memrchr(b'\n', &bytes[..search_end]) {
        newline_count += 1;
        if newline_count == remaining_newlines {
            return ReversePageChunk {
                start_byte: Some(absolute_start + index + 1),
                logical_bytes_examined: bytes.len() - index,
                newline_count,
            };
        }
        search_end = index;
    }
    ReversePageChunk {
        start_byte: None,
        logical_bytes_examined: bytes.len(),
        newline_count,
    }
}

struct PageScanState {
    start_byte: usize,
    offset: usize,
    lines_remaining: usize,
    lines: LineScanState,
    carry: Vec<u8>,
    newline_offsets: Vec<usize>,
}

impl PageScanState {
    fn new(start_byte: usize, page_lines: usize) -> Self {
        Self {
            start_byte,
            offset: start_byte,
            lines_remaining: page_lines,
            lines: LineScanState::new(start_byte),
            carry: Vec::new(),
            newline_offsets: Vec::new(),
        }
    }

    fn scan_chunk(&mut self, bytes: &[u8]) -> io::Result<bool> {
        let page_chunk = page_chunk(
            bytes,
            self.offset,
            self.lines_remaining,
            &mut self.newline_offsets,
        );
        let bytes = &bytes[..page_chunk.used];
        self.lines_remaining = self
            .lines_remaining
            .saturating_sub(self.newline_offsets.len());
        let carry_len = self.carry.len();
        let text_start_offset = self.offset - carry_len;
        if self.carry.is_empty() && bytes.is_ascii() {
            self.lines.scan_ascii_bytes_with_newlines(
                bytes,
                text_start_offset,
                &self.newline_offsets,
            );
            self.offset += bytes.len();
            return Ok(page_chunk.page_complete);
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
        self.lines.scan_valid_text_with_newlines(
            valid_text,
            text_start_offset,
            &self.newline_offsets,
        );
        self.carry.extend_from_slice(&text_bytes[valid_end..]);
        self.offset += bytes.len();
        Ok(page_chunk.page_complete)
    }

    fn finish_complete_page(&mut self) -> io::Result<()> {
        if !self.carry.is_empty() {
            return Err(incomplete_utf8_error());
        }
        self.lines.finish_complete_page();
        Ok(())
    }

    fn finish_final_page(&mut self) -> io::Result<()> {
        if !self.carry.is_empty() {
            return Err(incomplete_utf8_error());
        }
        self.lines.finish_final_page();
        Ok(())
    }

    fn into_scan(self, next_page_start: Option<usize>) -> PageScan {
        PageScan {
            lines: self.lines.into_scan(self.offset - self.start_byte),
            start_byte: self.start_byte,
            end_byte: self.offset,
            next_page_start,
        }
    }
}

struct PageChunk {
    used: usize,
    page_complete: bool,
}

fn page_chunk(
    bytes: &[u8],
    absolute_start: usize,
    remaining: usize,
    newline_offsets: &mut Vec<usize>,
) -> PageChunk {
    debug_assert!(remaining > 0);
    newline_offsets.clear();
    let mut search_start = 0usize;
    while let Some(relative_index) = memchr(b'\n', &bytes[search_start..]) {
        let index = search_start + relative_index;
        newline_offsets.push(absolute_start + index);
        if newline_offsets.len() == remaining {
            return PageChunk {
                used: index + 1,
                page_complete: true,
            };
        }
        search_start = index + 1;
    }
    PageChunk {
        used: bytes.len(),
        page_complete: false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::large_file::LINE_CHECKPOINT_INTERVAL_CHARS;
    use crate::config::big_files::DEFAULT_PAGE_LINES;

    #[test]
    fn forward_cutoff_does_not_validate_bytes_after_the_page_newline() {
        let bytes = [b'\n', 0xff, b'x'];

        let page = scan_utf8_page_bytes_for_perf(&bytes, 0, 1)
            .expect("the selected page prefix is valid UTF-8");

        assert_eq!(page.start_byte, 0);
        assert_eq!(page.end_byte, 1);
        assert_eq!(page.next_page_start, Some(1));
        assert_eq!(page.lines.line_starts, vec![0]);
        assert_eq!(page.lines.line_char_counts, vec![0]);
        assert_eq!(page.lines.line_is_ascii, vec![true]);
        assert!(scan_utf8_page_bytes_for_perf(&bytes, 1, 1).is_err());
    }

    #[test]
    fn shared_ascii_newlines_preserve_crlf_and_checkpoint_metadata() {
        let prefix = "a".repeat(LINE_CHECKPOINT_INTERVAL_CHARS + 7);
        let bytes = format!("{prefix}\r\ntail").into_bytes();

        let page = scan_utf8_page_bytes_for_perf(&bytes, 0, 1).expect("scan ASCII CRLF page");

        assert_eq!(page.end_byte, prefix.len() + 2);
        assert_eq!(page.next_page_start, Some(prefix.len() + 2));
        assert_eq!(page.lines.line_char_counts, vec![prefix.len()]);
        assert_eq!(page.lines.line_is_ascii, vec![true]);
        assert_eq!(page.lines.crlf_offsets, vec![prefix.len()]);
        assert_eq!(
            page.lines.line_checkpoints,
            vec![super::super::LineCheckpoint {
                col: LINE_CHECKPOINT_INTERVAL_CHARS,
                byte_offset: LINE_CHECKPOINT_INTERVAL_CHARS,
            }]
        );
    }

    #[test]
    fn crlf_split_at_read_boundary_keeps_exact_page_metadata() {
        let mut bytes = vec![b'a'; SCAN_CHUNK_BYTES - 1];
        bytes.extend_from_slice(b"\r\ntail");

        let page = scan_utf8_page_bytes_for_perf(&bytes, 0, 1)
            .expect("scan CRLF split across descriptor chunks");

        assert_eq!(page.end_byte, SCAN_CHUNK_BYTES + 1);
        assert_eq!(page.next_page_start, Some(SCAN_CHUNK_BYTES + 1));
        assert_eq!(page.lines.line_char_counts, vec![SCAN_CHUNK_BYTES - 1]);
        assert_eq!(page.lines.crlf_offsets, vec![SCAN_CHUNK_BYTES - 1]);
        assert_eq!(
            find_previous_page_start_bytes_for_perf(&bytes, page.end_byte, 1)
                .expect("locate the first page from its successor"),
            0
        );
    }

    #[test]
    fn default_page_scan_preserves_utf8_split_across_read_boundary() {
        let pattern = "a🙂\n";
        let mut text = pattern.repeat(DEFAULT_PAGE_LINES);
        text.push_str("tail");

        let page = scan_utf8_page_bytes_for_perf(text.as_bytes(), 0, DEFAULT_PAGE_LINES)
            .expect("scan the default-size Unicode page");

        let expected_end = pattern.len() * DEFAULT_PAGE_LINES;
        assert_eq!(page.end_byte, expected_end);
        assert_eq!(page.next_page_start, Some(expected_end));
        assert_eq!(page.lines.line_starts.len(), DEFAULT_PAGE_LINES);
        assert!(page.lines.line_char_counts.iter().all(|count| *count == 2));
        assert!(page.lines.line_is_ascii.iter().all(|is_ascii| !is_ascii));
        assert_eq!(
            find_previous_page_start_bytes_for_perf(
                text.as_bytes(),
                expected_end,
                DEFAULT_PAGE_LINES,
            )
            .expect("locate the page before the Unicode tail"),
            0
        );
    }

    #[test]
    fn forward_and_reverse_boundaries_match_scalar_model() {
        let mut bytes = vec![b'\n'];
        bytes.resize(SCAN_CHUNK_BYTES - 1, b'a');
        bytes.push(b'\n');
        bytes.push(b'\n');
        bytes.resize((SCAN_CHUNK_BYTES * 2) - 1, b'b');
        bytes.push(b'\n');
        bytes.extend_from_slice("é\r\nlast\n".as_bytes());

        for page_lines in [1, 2, 3] {
            let mut starts = vec![0usize];
            loop {
                let start = *starts.last().expect("the first page start is present");
                let page = scan_utf8_page_bytes_for_perf(&bytes, start, page_lines)
                    .expect("scan model page");
                let expected = scalar_page_boundary(&bytes, start, page_lines);
                assert_eq!(page.end_byte, expected.unwrap_or(bytes.len()));
                assert_eq!(page.next_page_start, expected);
                let Some(next) = page.next_page_start else {
                    break;
                };
                assert!(next > start);
                starts.push(next);
            }

            for pair in starts.windows(2) {
                let previous = find_previous_page_start_bytes_for_perf(&bytes, pair[1], page_lines)
                    .expect("reverse model page");
                assert_eq!(previous, pair[0]);
            }
        }
    }

    fn scalar_page_boundary(bytes: &[u8], start: usize, page_lines: usize) -> Option<usize> {
        bytes[start..]
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .nth(page_lines - 1)
            .map(|(offset, _)| start + offset + 1)
    }
}
