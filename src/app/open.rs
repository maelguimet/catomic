//! Open-size guardrail extraction + initial snapshot/open plan for App::new (Phase 2B).
//!
//! Purpose: encapsulate pre-read size policy (Large warn, Huge/Extreme page),
//!   one pinned regular-file descriptor (disk_snapshot + content plan), and
//!   the current content-plan-to-buffer construction seam.
//! Owns: prepare_open_file_meta (OpenSizeDecision + pinned descriptor snapshot;
//!   derives size/tier/content_plan from that descriptor) and build_open_buffer
//!   (editable PieceTable vs editable paged file buffer from the same descriptor).
//! Must not: construct watcher, mutate App, change snapshot/dirty/save/reload
//!   semantics beyond carrying the initial snapshot/plan and constructing the
//!   initial buffer, know terminal/render, or Project/LLM.
//! Invariants: identical observable outcomes for all documented App::new cases
//!   (None, missing, Small, Large, Huge/Extreme paged, hard meta error,
//!   invalid UTF-8 errors from read after successful metadata); non-regular paths
//!   are refused before buffer reads; one bounded identity capture drives
//!   size/snapshot/content planning; pathname drift fails closed.
//! Phase: 2-bm configurable paged open policy.

use std::io::{self, ErrorKind};

use crate::buffer::{self, Buffer};
use crate::file::io::{FileSnapshot, PinnedFile};
use crate::file::size::{
    classify_file_size, open_size_decision, open_size_warning_message, FileSizeTier,
    OpenSizeDecision,
};
use crate::file::text_format::TextFormat;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OpenContentPlan {
    /// No path was requested. Start with an untitled empty buffer.
    #[default]
    UntitledEmpty,
    /// The requested path was absent during the initial metadata capture.
    MissingEmpty,
    /// The requested path was present and must be read fully into the current buffer.
    FullRead,
    /// The requested path was oversized and opens one editable line page at a time.
    PagedEditable,
}

/// Captured pre-buffer-read decision for an optional path.
/// size_* are None for no-path or missing (Absent).
/// initial_message is Some for Large/Huge/Extreme files that warn on first open.
/// disk_snapshot carries the single initial capture (None for no path;
/// Absent for missing path; Present for existing) so App::new does not
/// probe metadata twice.
/// content_plan is the explicit current storage policy: empty for no-path/missing,
/// full read for Small/Large files; editable pages for Huge/Extreme files.
/// It is the narrow seam for future lazy storage work.
#[derive(Debug, Default)]
pub(crate) struct OpenFileMeta {
    pub size_bytes: Option<u64>,
    pub size_tier: Option<FileSizeTier>,
    pub initial_message: Option<String>,
    pub disk_snapshot: Option<FileSnapshot>,
    pub content_plan: OpenContentPlan,
    pub text_format: TextFormat,
    source: Option<PinnedFile>,
}

/// Capture bounded on-disk identity once and apply open-size guardrails. The
/// snapshot fully hashes files through 100 MiB and samples paged files; one
/// capture supplies both the size decision and App::new disk snapshot.
///
/// - None path: default (snapshot=None, no size, no message).
/// - Missing: sizes=None, disk_snapshot=Some(Absent); caller opens empty.
/// - Existing Small: size+Small from snapshot, no message, snapshot=Present.
/// - Existing Large: editable full read with an initial warning.
/// - Existing Huge/Extreme: editable paged storage with an initial warning.
/// - Hard meta error: propagates Err.
///
/// Does not build a buffer/App or touch the watcher.
pub(crate) fn prepare_open_file_meta(initial_path: Option<&str>) -> io::Result<OpenFileMeta> {
    let mut meta = OpenFileMeta::default();
    if let Some(p) = initial_path {
        match PinnedFile::open(p)? {
            Some(mut source) => {
                let snap = source.snapshot().clone();
                if let FileSnapshot::Present { len, .. } = &snap {
                    let sz = *len;
                    match open_size_decision(sz) {
                        OpenSizeDecision::Warn => {
                            meta.size_bytes = Some(sz);
                            let tier = classify_file_size(sz);
                            meta.size_tier = Some(tier);
                            meta.initial_message = open_size_warning_message(sz, tier);
                            meta.content_plan = OpenContentPlan::FullRead;
                        }
                        OpenSizeDecision::Paged => {
                            meta.text_format = crate::file::text_format::detect_file_format_from(
                                source.file_mut(),
                            )?;
                            meta.size_bytes = Some(sz);
                            let tier = classify_file_size(sz);
                            meta.size_tier = Some(tier);
                            meta.initial_message = open_size_warning_message(sz, tier);
                            meta.content_plan = OpenContentPlan::PagedEditable;
                        }
                        OpenSizeDecision::Normal => {
                            meta.size_bytes = Some(sz);
                            meta.size_tier = Some(classify_file_size(sz));
                            meta.content_plan = OpenContentPlan::FullRead;
                        }
                    }
                }
                meta.disk_snapshot = Some(snap);
                meta.source = Some(source);
            }
            None => {
                meta.content_plan = OpenContentPlan::MissingEmpty;
                meta.disk_snapshot = Some(FileSnapshot::Absent);
            }
        }
    }
    Ok(meta)
}

/// Construct the initial buffer selected by OpenContentPlan.
/// This is the current storage policy seam:
/// - no path / missing path => empty PieceTable
/// - Small/Large present path => full UTF-8 read + owned PieceTable construction
/// - Huge/Extreme present path => configured editable PagedFileBuffer page
///
/// Future lazy/partial storage work should branch here (or below the PieceTable
/// constructor) without adding file I/O to the buffer module or App::new.
pub(crate) fn build_open_buffer(
    meta: &mut OpenFileMeta,
    initial_path: Option<&str>,
    page_lines: usize,
) -> io::Result<Box<dyn Buffer>> {
    match meta.content_plan {
        OpenContentPlan::UntitledEmpty | OpenContentPlan::MissingEmpty => {
            Ok(Box::new(buffer::PieceTable::new()))
        }
        OpenContentPlan::FullRead => {
            let path = required_path(initial_path, "FullRead")?;
            let mut source = take_source(meta, path)?;
            // Move the read buffer into PieceTable on open; this avoids cloning
            // Large/Huge files while preserving CRLF normalization inside PT.
            let bytes = source.read_all_verified(path)?;
            let decoded = crate::file::text_format::decode(bytes)?;
            source.ensure_path_unchanged(path)?;
            meta.text_format = decoded.format;
            Ok(Box::new(buffer::PieceTable::from_owned_text(decoded.text)))
        }
        OpenContentPlan::PagedEditable => {
            let path = required_path(initial_path, "PagedEditable")?;
            if meta.text_format.utf8_bom
                || meta.text_format.line_ending == crate::file::text_format::LineEnding::Cr
            {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "UTF-8 BOM and CR-only files must be opened below the paged-file threshold",
                ));
            }
            let mut source = take_source(meta, path)?;
            source.ensure_descriptor_unchanged(path)?;
            let snapshot = source.snapshot().clone();
            let buffer = buffer::PagedFileBuffer::from_file(source.into_file(), page_lines)?;
            crate::file::io::ensure_path_matches_snapshot(path, &snapshot)?;
            Ok(Box::new(buffer))
        }
    }
}

fn required_path<'a>(initial_path: Option<&'a str>, plan: &str) -> io::Result<&'a std::path::Path> {
    initial_path.map(std::path::Path::new).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{plan} open plan requires initial path"),
        )
    })
}

fn take_source(meta: &mut OpenFileMeta, path: &std::path::Path) -> io::Result<PinnedFile> {
    let source = meta.source.take().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("open plan missing pinned descriptor: {}", path.display()),
        )
    })?;
    if meta.disk_snapshot.as_ref() != Some(source.snapshot()) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "open plan snapshot does not match descriptor: {}",
                path.display()
            ),
        ));
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("catomic_open_plan_{}_{}", std::process::id(), name));
        p
    }

    fn cleanup(path: &std::path::Path) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn no_path_plans_untitled_empty() {
        let meta = prepare_open_file_meta(None).unwrap();

        assert_eq!(meta.content_plan, OpenContentPlan::UntitledEmpty);
        assert!(meta.disk_snapshot.is_none());
        assert!(meta.size_bytes.is_none());
        assert!(meta.size_tier.is_none());
        assert!(meta.initial_message.is_none());
    }

    #[test]
    fn missing_path_plans_missing_empty_from_absent_snapshot() {
        let path = temp_path("missing.txt");
        cleanup(&path);

        let meta = prepare_open_file_meta(Some(&path.to_string_lossy())).unwrap();

        assert_eq!(meta.content_plan, OpenContentPlan::MissingEmpty);
        assert_eq!(meta.disk_snapshot, Some(FileSnapshot::Absent));
        assert!(meta.size_bytes.is_none());
        assert!(meta.size_tier.is_none());
        assert!(meta.initial_message.is_none());
    }

    #[test]
    fn present_path_plans_full_read_and_derives_size_from_snapshot() {
        let path = temp_path("present.txt");
        cleanup(&path);
        fs::write(&path, "hello\n").unwrap();

        let meta = prepare_open_file_meta(Some(&path.to_string_lossy())).unwrap();

        assert_eq!(meta.content_plan, OpenContentPlan::FullRead);
        assert_eq!(meta.size_bytes, Some(6));
        assert_eq!(meta.size_tier, Some(FileSizeTier::Small));
        match meta.disk_snapshot {
            Some(FileSnapshot::Present { len, .. }) => assert_eq!(len, 6),
            other => panic!("present path must carry Present snapshot, got {:?}", other),
        }
        assert!(meta.initial_message.is_none());

        cleanup(&path);
    }

    #[cfg(unix)]
    #[test]
    fn fifo_path_is_refused_before_content_read() {
        let path = temp_path("blocking.fifo");
        cleanup(&path);
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("run mkfifo");
        assert!(status.success());

        let error = prepare_open_file_meta(Some(&path.to_string_lossy()))
            .expect_err("FIFO open must fail without reading it");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains("non-regular"));
        cleanup(&path);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_regular_file_remains_openable() {
        let target = temp_path("symlink-target.txt");
        let link = temp_path("symlink.txt");
        cleanup(&target);
        cleanup(&link);
        fs::write(&target, "hello").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut meta = prepare_open_file_meta(Some(&link.to_string_lossy())).unwrap();
        let buffer = build_open_buffer(&mut meta, Some(&link.to_string_lossy()), 20_000).unwrap();

        assert_eq!(meta.content_plan, OpenContentPlan::FullRead);
        assert_eq!(buffer.to_string(), "hello");
        cleanup(&link);
        cleanup(&target);
    }

    #[test]
    fn huge_path_plans_editable_pages_from_snapshot() {
        let meta = OpenFileMeta {
            size_bytes: Some(crate::file::size::LARGE_FILE_LIMIT_BYTES + 1),
            size_tier: Some(FileSizeTier::Huge),
            initial_message: open_size_warning_message(
                crate::file::size::LARGE_FILE_LIMIT_BYTES + 1,
                FileSizeTier::Huge,
            ),
            disk_snapshot: Some(FileSnapshot::Present {
                len: crate::file::size::LARGE_FILE_LIMIT_BYTES + 1,
                mtime: None,
                change_id: None,
                content_identity: None,
            }),
            content_plan: OpenContentPlan::PagedEditable,
            ..OpenFileMeta::default()
        };

        assert_eq!(meta.content_plan, OpenContentPlan::PagedEditable);
        assert!(meta
            .initial_message
            .as_deref()
            .unwrap_or("")
            .contains("editable paged"));
    }

    #[test]
    fn build_open_buffer_empty_plans_start_empty() {
        let mut no_path = OpenFileMeta {
            content_plan: OpenContentPlan::UntitledEmpty,
            ..OpenFileMeta::default()
        };
        let mut missing = OpenFileMeta {
            content_plan: OpenContentPlan::MissingEmpty,
            ..OpenFileMeta::default()
        };

        let untitled = build_open_buffer(&mut no_path, None, 20_000).unwrap();
        let missing_buf = build_open_buffer(&mut missing, Some("missing.txt"), 20_000).unwrap();

        assert_eq!(untitled.to_string(), "");
        assert_eq!(missing_buf.to_string(), "");
        assert_eq!(untitled.line_count(), 1);
        assert_eq!(missing_buf.line_count(), 1);
    }

    #[test]
    fn build_open_buffer_full_read_moves_present_content_into_piece_table() {
        let path = temp_path("build_present.txt");
        cleanup(&path);
        fs::write(&path, "hello\nworld").unwrap();
        let mut meta = prepare_open_file_meta(Some(&path.to_string_lossy())).unwrap();

        let buffer = build_open_buffer(&mut meta, Some(&path.to_string_lossy()), 20_000).unwrap();

        assert_eq!(buffer.to_string(), "hello\nworld");
        assert_eq!(buffer.line_count(), 2);

        cleanup(&path);
    }

    #[test]
    fn build_open_buffer_full_read_requires_path() {
        let mut meta = OpenFileMeta {
            content_plan: OpenContentPlan::FullRead,
            ..OpenFileMeta::default()
        };

        let err = match build_open_buffer(&mut meta, None, 20_000) {
            Ok(_) => panic!("FullRead must require path"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_open_buffer_editable_pages_use_configured_line_count() {
        let path = temp_path("build_editable_pages.txt");
        cleanup(&path);
        fs::write(&path, "first\nsecond").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(crate::file::size::LARGE_FILE_LIMIT_BYTES + 1)
            .unwrap();
        let mut meta = prepare_open_file_meta(Some(&path.to_string_lossy())).unwrap();

        let buffer = build_open_buffer(&mut meta, Some(&path.to_string_lossy()), 1).unwrap();

        assert!(!buffer.is_read_only());
        assert_eq!(buffer.line_count(), 1);
        assert_eq!(buffer.line(0).as_deref(), Some("first"));
        assert!(buffer.page_info().unwrap().has_next);

        cleanup(&path);
    }

    #[cfg(unix)]
    #[test]
    fn regular_file_to_fifo_swap_fails_closed_without_reading_fifo() {
        let path = temp_path("regular_to_fifo.txt");
        cleanup(&path);
        fs::write(&path, "pinned bytes").unwrap();
        let mut meta = prepare_open_file_meta(Some(&path.to_string_lossy())).unwrap();

        cleanup(&path);
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("run mkfifo");
        assert!(status.success());

        let error = match build_open_buffer(&mut meta, Some(&path.to_string_lossy()), 20_000) {
            Ok(_) => panic!("pathname drift must reject the pinned startup load"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), ErrorKind::Interrupted);
        cleanup(&path);
    }
}
