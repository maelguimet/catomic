//! Open-size guardrail extraction + initial snapshot capture for App::new (Phase 2B).
//!
//! Purpose: encapsulate pre-read size guardrails (Extreme refuse, Large/Huge warn)
//!   and the single initial metadata capture (disk_snapshot) so App::new performs
//!   only one fs::metadata probe for present files.
//! Owns: prepare_open_file_meta (OpenSizeDecision + capture_file_snapshot once;
//!   derives size/tier from the snapshot for Present).
//! Must not: perform the content read_to_string, construct watcher, change
//!   snapshot/dirty/save/reload semantics beyond carrying the initial snapshot,
//!   know terminal/render, or Project/LLM.
//! Invariants: identical observable outcomes for all documented App::new cases
//!   (None, missing, Small, Large/Huge, Extreme refuse before read, hard meta error,
//!   invalid UTF-8 errors from read after successful small metadata); single capture
//!   for size + snapshot on the present-file path.
//! Phase: 2-aj extraction + 2-am single-capture hygiene (behavior identical).

use std::io::{self, ErrorKind};

use crate::file::io::FileSnapshot;
use crate::file::size::{
    classify_file_size, open_size_decision, open_size_refusal_message, open_size_warning_message,
    FileSizeTier, OpenSizeDecision,
};

/// Captured pre-read metadata decision for an optional path.
/// size_* are None for no-path or missing (Absent).
/// initial_message is Some only for Large/Huge that should warn on first open.
/// disk_snapshot carries the single initial capture (None for no path;
/// Absent for missing path; Present for existing) so App::new does not
/// probe metadata twice.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpenFileMeta {
    pub size_bytes: Option<u64>,
    pub size_tier: Option<FileSizeTier>,
    pub initial_message: Option<String>,
    pub disk_snapshot: Option<FileSnapshot>,
}

/// Probe on-disk metadata once (via capture_file_snapshot) and apply open-size
/// guardrails. Single capture populates both size decision and the disk_snapshot
/// carried back to App::new (avoids duplicate metadata probe for present files).
/// - None path: default (snapshot=None, no size, no message).
/// - Missing: sizes=None, disk_snapshot=Some(Absent); caller reads "".
/// - Existing Small: size+Small from snapshot, no message, snapshot=Present.
/// - Existing Large/Huge: size+tier from snapshot, initial_message=warning.
/// - Extreme: Err(InvalidData) before returning meta (before any content read).
/// - Hard meta error: propagates Err.
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
                        OpenSizeDecision::Refuse => {
                            let msg = open_size_refusal_message(sz);
                            return Err(io::Error::new(ErrorKind::InvalidData, msg));
                        }
                        OpenSizeDecision::OpenWithWarning => {
                            meta.size_bytes = Some(sz);
                            let tier = classify_file_size(sz);
                            meta.size_tier = Some(tier);
                            meta.initial_message = open_size_warning_message(sz, tier);
                        }
                        OpenSizeDecision::OpenNormally => {
                            meta.size_bytes = Some(sz);
                            meta.size_tier = Some(classify_file_size(sz));
                        }
                    }
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
