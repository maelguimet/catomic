//! Purpose: expose process primitives used by explicit commands, linters, and link opening.
//! Owns: bounded command tasks and detached system-opener handoff boundaries.
//! Must not: load configuration, dispatch editor actions, or mutate buffers/files.
//! Invariants: callers only receive bounded, polled task results.

mod open_link;
mod task;

pub(crate) use open_link::open_http_link;
pub(crate) use task::{ExternalCommandResult, ExternalCommandTask};

pub(crate) fn substitute_file(template: &str, path: &std::path::Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "'\"'\"'");
    template.replace("{file}", &format!("'{escaped}'"))
}
