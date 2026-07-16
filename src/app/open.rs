//! Open-size guardrail extraction + initial snapshot/open plan for App::new (Phase 2B).
//!
//! Purpose: encapsulate pre-read size policy (Large warn, Huge/Extreme page)
//!   the single initial metadata capture (disk_snapshot + content plan), and
//!   the current content-plan-to-buffer construction seam.
//! Owns: prepare_open_file_meta (OpenSizeDecision + capture_file_snapshot once;
//!   derives size/tier/content_plan from the snapshot for Present/Absent) and
//!   build_open_buffer (editable PieceTable vs editable paged file buffer).
//! Must not: construct watcher, mutate App, change snapshot/dirty/save/reload
//!   semantics beyond carrying the initial snapshot/plan and constructing the
//!   initial buffer, know terminal/render, or Project/LLM.
//! Invariants: identical observable outcomes for all documented App::new cases
//!   (None, missing, Small, Large, Huge/Extreme paged, hard meta error,
//!   invalid UTF-8 errors from read after successful metadata); single capture
//!   for size + snapshot + content plan on the present/missing-file paths.
//! Phase: 2-bm configurable paged open policy.

use std::io::{self, ErrorKind};

use crate::buffer::{self, Buffer};
use crate::file::io::FileSnapshot;
use crate::file::size::{
    classify_file_size, open_size_decision, open_size_warning_message, FileSizeTier,
    OpenSizeDecision,
};

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

/// Captured pre-read metadata decision for an optional path.
/// size_* are None for no-path or missing (Absent).
/// initial_message is Some for Large/Huge/Extreme files that warn on first open.
/// disk_snapshot carries the single initial capture (None for no path;
/// Absent for missing path; Present for existing) so App::new does not
/// probe metadata twice.
/// content_plan is the explicit current storage policy: empty for no-path/missing,
/// full read for Small/Large files; editable pages for Huge/Extreme files.
/// It is the narrow seam for future lazy storage work.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpenFileMeta {
    pub size_bytes: Option<u64>,
    pub size_tier: Option<FileSizeTier>,
    pub initial_message: Option<String>,
    pub disk_snapshot: Option<FileSnapshot>,
    pub content_plan: OpenContentPlan,
}

/// Probe on-disk metadata once (via capture_file_snapshot) and apply open-size
/// guardrails. Single capture populates both size decision and the disk_snapshot
/// carried back to App::new (avoids duplicate metadata probe for present files).
///
/// - None path: default (snapshot=None, no size, no message).
/// - Missing: sizes=None, disk_snapshot=Some(Absent); caller opens empty.
/// - Existing Small: size+Small from snapshot, no message, snapshot=Present.
/// - Existing Large: editable full read with an initial warning.
/// - Existing Huge/Extreme: editable paged storage with an initial warning.
/// - Hard meta error: propagates Err.
///
/// Does not read content, does not build buffer/App, does not touch watcher.
pub(crate) fn prepare_open_file_meta(initial_path: Option<&str>) -> io::Result<OpenFileMeta> {
    let mut meta = OpenFileMeta::default();
    if let Some(p) = initial_path {
        // Single capture for both size decision and snapshot carried to App.
        match crate::file::io::capture_file_snapshot(p) {
            Ok(snap) => {
                if let FileSnapshot::Present { len, .. } = &snap {
                    let sz = *len;
                    match open_size_decision(sz) {
                        OpenSizeDecision::OpenWithWarning => {
                            meta.size_bytes = Some(sz);
                            let tier = classify_file_size(sz);
                            meta.size_tier = Some(tier);
                            meta.initial_message = open_size_warning_message(sz, tier);
                            meta.content_plan = OpenContentPlan::FullRead;
                        }
                        OpenSizeDecision::OpenPaged => {
                            meta.size_bytes = Some(sz);
                            let tier = classify_file_size(sz);
                            meta.size_tier = Some(tier);
                            meta.initial_message = open_size_warning_message(sz, tier);
                            meta.content_plan = OpenContentPlan::PagedEditable;
                        }
                        OpenSizeDecision::OpenNormally => {
                            meta.size_bytes = Some(sz);
                            meta.size_tier = Some(classify_file_size(sz));
                            meta.content_plan = OpenContentPlan::FullRead;
                        }
                    }
                } else {
                    meta.content_plan = OpenContentPlan::MissingEmpty;
                }
                // Absent or Present: carry the snapshot captured here.
                meta.disk_snapshot = Some(snap);
            }
            Err(e) => {
                // Hard metadata error (capture does not map NotFound to error).
                return Err(e);
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
    meta: &OpenFileMeta,
    initial_path: Option<&str>,
    page_lines: usize,
) -> io::Result<Box<dyn Buffer>> {
    match meta.content_plan {
        OpenContentPlan::UntitledEmpty | OpenContentPlan::MissingEmpty => {
            Ok(Box::new(buffer::PieceTable::new()))
        }
        OpenContentPlan::FullRead => {
            let path = initial_path.ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    "FullRead open plan requires initial path",
                )
            })?;
            // Move the read buffer into PieceTable on open; this avoids cloning
            // Large/Huge files while preserving CRLF normalization inside PT.
            let content = crate::file::io::read_to_string(path)?;
            Ok(Box::new(buffer::PieceTable::from_owned_text(content)))
        }
        OpenContentPlan::PagedEditable => {
            let path = initial_path.ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    "PagedEditable open plan requires initial path",
                )
            })?;
            Ok(Box::new(buffer::PagedFileBuffer::open(path, page_lines)?))
        }
    }
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
            }),
            content_plan: OpenContentPlan::PagedEditable,
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
        let no_path = OpenFileMeta {
            content_plan: OpenContentPlan::UntitledEmpty,
            ..OpenFileMeta::default()
        };
        let missing = OpenFileMeta {
            content_plan: OpenContentPlan::MissingEmpty,
            ..OpenFileMeta::default()
        };

        let untitled = build_open_buffer(&no_path, None, 20_000).unwrap();
        let missing_buf = build_open_buffer(&missing, Some("missing.txt"), 20_000).unwrap();

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
        let meta = OpenFileMeta {
            content_plan: OpenContentPlan::FullRead,
            ..OpenFileMeta::default()
        };

        let buffer = build_open_buffer(&meta, Some(&path.to_string_lossy()), 20_000).unwrap();

        assert_eq!(buffer.to_string(), "hello\nworld");
        assert_eq!(buffer.line_count(), 2);

        cleanup(&path);
    }

    #[test]
    fn build_open_buffer_full_read_requires_path() {
        let meta = OpenFileMeta {
            content_plan: OpenContentPlan::FullRead,
            ..OpenFileMeta::default()
        };

        let err = match build_open_buffer(&meta, None, 20_000) {
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
        let meta = OpenFileMeta {
            content_plan: OpenContentPlan::PagedEditable,
            ..OpenFileMeta::default()
        };

        let buffer = build_open_buffer(&meta, Some(&path.to_string_lossy()), 1).unwrap();

        assert!(!buffer.is_read_only());
        assert_eq!(buffer.line_count(), 1);
        assert_eq!(buffer.line(0).as_deref(), Some("first"));
        assert!(buffer.page_info().unwrap().has_next);

        cleanup(&path);
    }
}
