//! Open-size guardrail extraction for App::new (Phase 2B).
//!
//! Purpose: encapsulate the pre-read size probe, Extreme refusal, Large/Huge
//!   warning decision, and metadata capture so App::new stays focused and short.
//! Owns: prepare_open_file_meta (uses OpenSizeDecision + pure helpers from file::size).
//! Must not: perform the content read_to_string, construct watcher, change
//!   snapshot/dirty/save/reload semantics, know terminal/render, or Project/LLM.
//! Invariants: identical observable outcomes for all documented App::new cases
//!   (None, missing, Small, Large/Huge, Extreme refuse, hard meta error, utf8 error
//!   after small probe); uses existing OpenSizeDecision to avoid raw tier matches.
//! Phase: 2-aj (extract for hygiene + reviewability; behavior identical).

use std::io::{self, ErrorKind};

use crate::file::size::{
    classify_file_size, file_size_bytes, open_size_decision, open_size_refusal_message,
    open_size_warning_message, FileSizeTier, OpenSizeDecision,
};

/// Captured pre-read metadata decision for an optional path.
/// size_* are None for no-path or missing (Absent).
/// initial_message is Some only for Large/Huge that should warn on first open.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpenFileMeta {
    pub size_bytes: Option<u64>,
    pub size_tier: Option<FileSizeTier>,
    pub initial_message: Option<String>,
}

/// Probe on-disk size (metadata only) and apply open-size guardrails.
/// - None path: returns default (no size, no message).
/// - Missing (NotFound): returns default sizes; caller will read "" and snapshot Absent.
/// - Existing Small: size+Small recorded, no message.
/// - Existing Large/Huge: size+tier recorded, initial_message = warning.
/// - Extreme: returns Err(InvalidData with refusal) before any content read.
/// - Hard meta error (!NotFound): propagates the error.
/// Does not read content, does not build buffer/App, does not touch watcher.
pub(crate) fn prepare_open_file_meta(initial_path: Option<&str>) -> io::Result<OpenFileMeta> {
    let mut meta = OpenFileMeta::default();
    if let Some(p) = initial_path {
        match file_size_bytes(p) {
            Ok(sz) => {
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
                        // message stays None
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // Remember path but size None; read yields empty, snapshot will be Absent.
            }
            Err(e) => {
                // Hard metadata error other than NotFound: surface, do not guess.
                return Err(e);
            }
        }
    }
    Ok(meta)
}
