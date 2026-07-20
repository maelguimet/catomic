//! Purpose: this file must keep unconfirmed sensitive repository bytes out of LLM context.
//! Owns: dot-path admission, obvious secret detection, and explicit omission labels.
//! Must not: read files, mutate broker budgets, write, or network.
//! Invariants: dot paths stay outside the broker map and secret-like bytes are never returned.

use std::path::Path;

use crate::llm::context::{is_dotfile, secret_like};

use super::BrokerError;

pub(super) fn allow_path(path: &Path, omitted: &mut usize) -> bool {
    if is_dotfile(path) {
        *omitted += 1;
        false
    } else {
        true
    }
}

pub(super) fn reject_content(path: &Path, bytes: &[u8]) -> Result<(), BrokerError> {
    if has_secret_like(bytes) {
        Err(BrokerError::SensitiveContent(path.to_path_buf()))
    } else {
        Ok(())
    }
}

pub(super) fn has_secret_like(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes).lines().any(secret_like)
}

pub(super) fn file_map_note(truncated: bool, sensitive_omitted: usize) -> String {
    let mut notes = Vec::new();
    if truncated {
        notes.push("truncated".to_string());
    }
    if sensitive_omitted > 0 {
        notes.push(format!("{sensitive_omitted} sensitive dot paths omitted"));
    }
    if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join(", "))
    }
}

pub(super) fn append_grep_notice(matches: &mut String, sensitive_omitted: usize) {
    if sensitive_omitted > 0 {
        matches.push_str(&format!(
            "[broker omitted {sensitive_omitted} files with secret-like content]\n"
        ));
    }
}
