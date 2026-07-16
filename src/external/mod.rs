//! Purpose: expose the bounded process primitive used by explicit commands and linters.
//! Owns: external process task module boundaries.
//! Must not: load configuration, dispatch editor actions, or mutate buffers/files.
//! Invariants: callers only receive bounded, polled task results.
//! Phase: 7 external command foundation.

mod task;

pub(crate) use task::{ExternalCommandResult, ExternalCommandTask};

pub(crate) fn substitute_file(template: &str, path: &std::path::Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "'\"'\"'");
    template.replace("{file}", &format!("'{escaped}'"))
}
