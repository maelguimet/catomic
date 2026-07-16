//! Purpose: locate global logical lines in descriptor-backed paged documents.
//! Owns: bounded descriptor scanning, edited-page overlay accounting, and cancellation.
//! Must not: render, mutate App/Buffer state, reopen paths, or create idle workers.
//! Invariants: requested and reported lines are 1-based; positions retain source page identity.
//! Phase: 3-c global goto-line completion.

use std::io;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crate::buffer::{DescriptorOverlay, DescriptorPosition, DescriptorSource};

const GOTO_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GotoLineMatch {
    pub(crate) position: DescriptorPosition,
    pub(crate) line: usize,
}

pub(crate) enum GotoLineResult {
    Found(GotoLineMatch),
    Error(String),
}

pub(crate) struct GotoLineTask {
    receiver: mpsc::Receiver<GotoLineResult>,
    cancel: Arc<AtomicBool>,
}

impl GotoLineTask {
    pub(crate) fn try_result(&self) -> Option<GotoLineResult> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => None,
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for GotoLineTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn start_descriptor_goto(
    source: DescriptorSource,
    requested_line: usize,
) -> GotoLineTask {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let result = scan_descriptor(source, requested_line, &worker_cancel)
            .map(GotoLineResult::Found)
            .unwrap_or_else(|error| GotoLineResult::Error(error.to_string()));
        if !worker_cancel.load(Ordering::Acquire) {
            let _ = sender.send(result);
        }
    });
    GotoLineTask { receiver, cancel }
}

fn scan_descriptor(
    source: DescriptorSource,
    requested_line: usize,
    cancel: &AtomicBool,
) -> io::Result<GotoLineMatch> {
    if requested_line == 0 || source.page_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "goto line and page size must be positive",
        ));
    }
    let initial_meta = source.file.metadata()?;
    if initial_meta.len() != source.total_bytes {
        return Err(changed_file_error());
    }
    let initial_modified = initial_meta.modified().ok();
    let mut scanner = LineScanner::new(requested_line, source.page_lines);
    if requested_line == 1 {
        ensure_unchanged(&source, initial_modified)?;
        return Ok(scanner.last_match());
    }

    let mut chunk = vec![0u8; GOTO_CHUNK_BYTES];
    let mut offset = 0u64;
    let mut overlay_index = 0usize;
    while offset < source.total_bytes {
        check_cancel(cancel)?;
        if let Some(overlay) = source.overlays.get(overlay_index) {
            validate_overlay(overlay, offset, source.total_bytes)?;
            if overlay.start_byte == offset {
                scanner.begin_page(overlay.start_byte, overlay.page_number);
                if let Some(found) =
                    scanner.scan_overlay(overlay, overlay.end_byte < source.total_bytes, cancel)?
                {
                    ensure_unchanged(&source, initial_modified)?;
                    return Ok(found);
                }
                offset = overlay.end_byte;
                scanner.begin_page(offset, overlay.page_number + 1);
                overlay_index += 1;
                continue;
            }
        }
        let read_limit = source
            .overlays
            .get(overlay_index)
            .map_or(chunk.len(), |overlay| {
                usize::try_from(overlay.start_byte - offset)
                    .unwrap_or(chunk.len())
                    .min(chunk.len())
            });
        let read = source.file.read_at(&mut chunk[..read_limit], offset)?;
        if read == 0 {
            return Err(changed_file_error());
        }
        if let Some(found) = scanner.scan_original(&chunk[..read], offset, cancel)? {
            ensure_unchanged(&source, initial_modified)?;
            return Ok(found);
        }
        offset += read as u64;
    }
    ensure_unchanged(&source, initial_modified)?;
    Ok(scanner.last_match())
}

struct LineScanner {
    requested_line: usize,
    page_lines: usize,
    line: usize,
    position: DescriptorPosition,
    last_line_position: DescriptorPosition,
}

impl LineScanner {
    fn new(requested_line: usize, page_lines: usize) -> Self {
        let position = DescriptorPosition {
            page_start: 0,
            page_number: 1,
            row: 0,
            col: 0,
        };
        Self {
            requested_line,
            page_lines,
            line: 1,
            position,
            last_line_position: position,
        }
    }

    fn begin_page(&mut self, page_start: u64, page_number: usize) {
        self.position.page_start = page_start;
        self.position.page_number = page_number;
        self.position.row = 0;
    }

    fn scan_original(
        &mut self,
        bytes: &[u8],
        start: u64,
        cancel: &AtomicBool,
    ) -> io::Result<Option<GotoLineMatch>> {
        for (index, byte) in bytes.iter().enumerate() {
            if index % 16_384 == 0 {
                check_cancel(cancel)?;
            }
            if *byte != b'\n' {
                continue;
            }
            self.line += 1;
            self.position.row += 1;
            if self.position.row == self.page_lines {
                self.begin_page(start + index as u64 + 1, self.position.page_number + 1);
            }
            self.last_line_position = self.position;
            if self.line == self.requested_line {
                return Ok(Some(self.last_match()));
            }
        }
        Ok(None)
    }

    fn scan_overlay(
        &mut self,
        overlay: &DescriptorOverlay,
        has_following_source: bool,
        cancel: &AtomicBool,
    ) -> io::Result<Option<GotoLineMatch>> {
        for (index, byte) in overlay.content.iter().enumerate() {
            if index % 16_384 == 0 {
                check_cancel(cancel)?;
            }
            if *byte != b'\n' {
                continue;
            }
            self.line += 1;
            self.position.row += 1;
            let ends_overlay = index + 1 == overlay.content.len();
            if ends_overlay && has_following_source {
                self.begin_page(overlay.end_byte, overlay.page_number + 1);
            }
            self.last_line_position = self.position;
            if self.line == self.requested_line {
                return Ok(Some(self.last_match()));
            }
        }
        Ok(None)
    }

    fn last_match(&self) -> GotoLineMatch {
        GotoLineMatch {
            position: self.last_line_position,
            line: self.line,
        }
    }
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "goto cancelled"))
    } else {
        Ok(())
    }
}

fn validate_overlay(overlay: &DescriptorOverlay, offset: u64, total: u64) -> io::Result<()> {
    if overlay.start_byte < offset
        || overlay.start_byte >= overlay.end_byte
        || overlay.end_byte > total
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid edited page range during goto",
        ))
    } else {
        Ok(())
    }
}

fn ensure_unchanged(
    source: &DescriptorSource,
    initial_modified: Option<std::time::SystemTime>,
) -> io::Result<()> {
    let meta = source.file.metadata()?;
    if meta.len() == source.total_bytes && meta.modified().ok() == initial_modified {
        Ok(())
    } else {
        Err(changed_file_error())
    }
}

fn changed_file_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "large file changed during goto")
}

#[cfg(test)]
mod tests;
