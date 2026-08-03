//! Purpose: scan one configured line page from a Huge-file descriptor.
//! Owns: bounded page boundaries, chunk-boundary UTF-8 validation, and page metadata.
//! Must not: reopen paths, retain descriptors, render, edit, or choose App policy.
//! Invariants: a complete non-final page ends after its configured newline count;
//!   page metadata uses absolute descriptor offsets and retains no file content.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

use super::scan::{LineScan, LineScanState};
use super::SCAN_CHUNK_BYTES;

pub(crate) struct PageScan {
    pub(crate) lines: LineScan,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) next_page_start: Option<usize>,
    #[cfg(test)]
    pub(crate) perf: PageScanPerfStats,
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
        #[cfg(test)]
        state.note_descriptor_read(n);
        let page_chunk = page_chunk(&chunk[..n], state.lines_remaining);
        #[cfg(test)]
        state.note_examined_bytes(page_chunk.used, page_chunk.newline_count);
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
        let page_chunk = page_chunk(chunk, state.lines_remaining);
        state.note_examined_bytes(page_chunk.used, page_chunk.newline_count);
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
    find_previous_page(file, current_start, page_lines).map(|scan| scan.start_byte)
}

#[cfg(test)]
pub(crate) fn find_previous_page_start_for_perf(
    file: &File,
    current_start: usize,
    page_lines: usize,
) -> io::Result<PreviousPageScan> {
    let scan = find_previous_page(file, current_start, page_lines)?;
    Ok(PreviousPageScan {
        start_byte: scan.start_byte,
        perf: scan.perf,
    })
}

#[cfg(test)]
pub(crate) fn find_previous_page_start_bytes_for_perf(
    bytes: &[u8],
    current_start: usize,
    page_lines: usize,
) -> io::Result<PreviousPageScan> {
    if current_start > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reverse page start exceeds in-memory fixture",
        ));
    }
    let target_newline = page_lines.saturating_add(1);
    let mut perf = PageScanPerfStats::default();
    let mut seen = 0usize;
    let mut end = current_start;
    while end > 0 {
        let start = end.saturating_sub(SCAN_CHUNK_BYTES);
        for index in (start..end).rev() {
            perf.logical_bytes_examined += 1;
            if bytes[index] == b'\n' {
                seen += 1;
                perf.newline_count += 1;
                if seen == target_newline {
                    return Ok(PreviousPageScan {
                        start_byte: index + 1,
                        perf,
                    });
                }
            }
        }
        end = start;
    }
    Ok(PreviousPageScan {
        start_byte: 0,
        perf,
    })
}

struct PreviousPageScanInternal {
    start_byte: usize,
    #[cfg(test)]
    perf: PageScanPerfStats,
}

fn find_previous_page(
    file: &File,
    current_start: usize,
    page_lines: usize,
) -> io::Result<PreviousPageScanInternal> {
    let target_newline = page_lines.saturating_add(1);
    let mut seen = 0usize;
    let mut end = current_start;
    let mut chunk = vec![0u8; SCAN_CHUNK_BYTES];
    #[cfg(test)]
    let mut perf = PageScanPerfStats::default();
    while end > 0 {
        let start = end.saturating_sub(chunk.len());
        let len = end - start;
        #[cfg(test)]
        let (read_calls, read_bytes) = read_exact_at(file, &mut chunk[..len], start)?;
        #[cfg(not(test))]
        read_exact_at(file, &mut chunk[..len], start)?;
        #[cfg(test)]
        {
            perf.descriptor_read_calls += read_calls;
            perf.descriptor_read_bytes += read_bytes;
        }
        for index in (0..len).rev() {
            #[cfg(test)]
            {
                perf.logical_bytes_examined += 1;
            }
            if chunk[index] == b'\n' {
                seen += 1;
                #[cfg(test)]
                {
                    perf.newline_count += 1;
                }
                if seen == target_newline {
                    return Ok(PreviousPageScanInternal {
                        start_byte: start + index + 1,
                        #[cfg(test)]
                        perf,
                    });
                }
            }
        }
        end = start;
    }
    Ok(PreviousPageScanInternal {
        start_byte: 0,
        #[cfg(test)]
        perf,
    })
}

#[cfg(not(test))]
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
fn read_exact_at(file: &File, mut out: &mut [u8], mut offset: usize) -> io::Result<(usize, usize)> {
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

struct PageScanState {
    start_byte: usize,
    offset: usize,
    lines_remaining: usize,
    lines: LineScanState,
    carry: Vec<u8>,
    #[cfg(test)]
    perf: PageScanPerfStats,
}

impl PageScanState {
    fn new(start_byte: usize, page_lines: usize) -> Self {
        Self {
            start_byte,
            offset: start_byte,
            lines_remaining: page_lines,
            lines: LineScanState::new(start_byte),
            carry: Vec::new(),
            #[cfg(test)]
            perf: PageScanPerfStats::default(),
        }
    }

    #[cfg(test)]
    fn note_descriptor_read(&mut self, bytes: usize) {
        self.perf.descriptor_read_calls += 1;
        self.perf.descriptor_read_bytes += bytes;
    }

    #[cfg(test)]
    fn note_examined_bytes(&mut self, bytes: usize, newlines: usize) {
        self.perf.logical_bytes_examined += bytes;
        self.perf.newline_count += newlines;
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
        self.lines.scan_valid_text(valid_text, text_start_offset);
        self.carry.extend_from_slice(&text_bytes[valid_end..]);
        self.offset += bytes.len();
        Ok(())
    }

    fn scan_ascii_bytes(&mut self, bytes: &[u8], text_start_offset: usize) {
        self.lines.scan_ascii_bytes(bytes, text_start_offset);
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
            #[cfg(test)]
            perf: self.perf,
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
