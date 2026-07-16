//! Manual reload-from-disk confirmation (Phase 2-s narrow pass).
//!
//! Purpose: owns the pending reload confirmation token, message helpers,
//! and the Ctrl+R decision + perform logic (extracted in 2-t for mod.rs hygiene).
//! Uses only metadata (ExternalFileStatus + FileSnapshot) via observe_external_file.
//! Owns: PendingReload struct, arm/perform helpers, handle_reload_key.
//! Must not: own watcher polling, background work, full content scans for
//!   *detection*, config parsing, Project, or LLM work.
//! Invariants: pending is bound to concrete (path + status + live snapshot);
//!   second press only acts on exact match; any content mutation clears it;
//!   automatic reload is invoked only for clean buffers by caller policy;
//!   movement/render do not clear pending state.
//! Phase: 2-s / 2-t cleanup through 2-bx automatic clean reload.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::buffer;
use crate::file::io::{
    observe_external_file, ExternalFileObservation, ExternalFileStatus, FileSnapshot,
};
use crate::file::size::{self, FileSizeTier, OpenSizeDecision};

/// Token recorded on first Ctrl+R when reload would change buffer state.
/// Binds to the specific observed disk state so that drift between presses
/// refuses the reload (similar to PendingSaveConflict).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingReload {
    /// Target path at arm time.
    pub path: PathBuf,
    pub status: ExternalFileStatus,
    /// Live snapshot (or None) at the time first Ctrl+R armed the confirmation.
    /// For Modified: must match exactly on second press.
    /// For Deleted: kind match sufficient.
    pub snapshot: Option<FileSnapshot>,
}

/// Returns the message for first Ctrl+R press that arms a reload confirmation.
pub(crate) fn reload_arm_message(status: &ExternalFileStatus, dirty: bool) -> String {
    match status {
        ExternalFileStatus::Modified => {
            if dirty {
                "File changed on disk. Press Ctrl+R again to reload from disk (discard local changes).".to_string()
            } else {
                "File changed on disk. Press Ctrl+R again to reload from disk.".to_string()
            }
        }
        ExternalFileStatus::Deleted => {
            if dirty {
                "File deleted on disk. Press Ctrl+R again to clear buffer (discard local changes)."
                    .to_string()
            } else {
                "File deleted on disk. Press Ctrl+R again to clear buffer.".to_string()
            }
        }
        _ => {
            // Should not arm for these; caller decides.
            format!("File status check failed: unexpected arm for {:?}", status)
        }
    }
}

/// Success message after actual reload of modified content.
pub(crate) fn reload_success_message() -> String {
    "Reloaded from disk.".to_string()
}

/// Success message after clearing buffer due to deleted file.
pub(crate) fn reload_cleared_message() -> String {
    "Buffer cleared (file deleted on disk).".to_string()
}

struct ReloadedModifiedBuffer {
    buffer: Box<dyn buffer::Buffer>,
    size_bytes: u64,
    size_tier: FileSizeTier,
}

fn observed_present_len(obs: &ExternalFileObservation) -> io::Result<u64> {
    match obs.live_snapshot {
        Some(FileSnapshot::Present { len, .. }) => Ok(len),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reload modified path missing present size snapshot",
        )),
    }
}

fn build_modified_reload_buffer(
    path: &Path,
    size_bytes: u64,
    page_lines: usize,
) -> io::Result<ReloadedModifiedBuffer> {
    let size_tier = size::classify_file_size(size_bytes);
    let buffer: Box<dyn buffer::Buffer> = match size::open_size_decision(size_bytes) {
        OpenSizeDecision::Paged => Box::new(buffer::PagedFileBuffer::open(path, page_lines)?),
        OpenSizeDecision::Normal | OpenSizeDecision::Warn => {
            let content = crate::file::io::read_to_string(path)?;
            Box::new(buffer::PieceTable::from_owned_text(content))
        }
    };

    Ok(ReloadedModifiedBuffer {
        buffer,
        size_bytes,
        size_tier,
    })
}

fn reload_modified_success_message(size_bytes: u64, size_tier: FileSizeTier) -> String {
    if matches!(size_tier, FileSizeTier::Huge | FileSizeTier::Extreme) {
        if let Some(warning) = size::open_size_warning_message(size_bytes, size_tier) {
            return format!("Reloaded from disk. {}", warning);
        }
    }
    reload_success_message()
}

/// Replace a clean buffer from one already-fresh Modified/Deleted observation.
/// Watcher policy and Ctrl+R confirmation both call this narrow mutation seam.
/// Errors are surfaced in `message` and leave the existing buffer intact.
pub(crate) fn perform_observed_reload(app: &mut super::App, obs: &ExternalFileObservation) {
    let Some(path) = app.file.path.clone() else {
        app.message = Some("No file path.".to_string());
        return;
    };
    match obs.status {
        ExternalFileStatus::Modified => {
            match observed_present_len(obs).and_then(|size_bytes| {
                build_modified_reload_buffer(&path, size_bytes, app.big_files.page_lines)
            }) {
                Ok(reloaded) => apply_modified_reload(app, &path, reloaded),
                Err(error) => app.message = Some(format!("Reload error: {error}")),
            }
        }
        ExternalFileStatus::Deleted => apply_deleted_reload(app),
        _ => apply_check_observation(app, obs),
    }
}

fn apply_modified_reload(app: &mut super::App, path: &Path, reloaded: ReloadedModifiedBuffer) {
    let reload_message = reload_modified_success_message(reloaded.size_bytes, reloaded.size_tier);
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::view::cancel_preview(app);
    app.selection.clear();
    app.buffer = reloaded.buffer;
    app.file.saved_history_position = app.buffer.edit_history_position();
    app.file.dirty = false;
    match crate::file::io::capture_file_snapshot(path) {
        Ok(snapshot @ FileSnapshot::Present { len, .. }) => {
            app.file.size_bytes = Some(len);
            app.file.size_tier = Some(crate::file::size::classify_file_size(len));
            app.file.disk_snapshot = Some(snapshot);
        }
        Ok(snapshot) => {
            app.file.disk_snapshot = Some(snapshot);
            app.file.size_bytes = None;
            app.file.size_tier = None;
        }
        Err(_) => {
            app.file.size_bytes = Some(reloaded.size_bytes);
            app.file.size_tier = Some(reloaded.size_tier);
        }
    }
    finish_reload(app, reload_message);
}

fn apply_deleted_reload(app: &mut super::App) {
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::view::cancel_preview(app);
    app.selection.clear();
    app.buffer = Box::new(buffer::PieceTable::new());
    app.file.saved_history_position = app.buffer.edit_history_position();
    app.file.dirty = false;
    app.file.disk_snapshot = Some(FileSnapshot::Absent);
    app.file.size_bytes = None;
    app.file.size_tier = None;
    finish_reload(app, reload_cleared_message());
}

fn finish_reload(app: &mut super::App, message: String) {
    app.message = Some(message);
    app.pending_reload = None;
    app.pending_save_conflict = None;
    app.pending_quit_confirm = false;
    app.reveal_cursor();
}

/// Apply a single ExternalFileObservation to set user message and arm/clear
/// pending_reload. This is the single-source status+arm path for manual check.
/// NoPath/Unchanged/Unknown: set message, clear pending.
/// Modified/Deleted: arm pending bound to obs.live_snapshot (for drift), set arm message.
/// Does not mutate buffer, dirty, disk_snapshot, or history.
pub(crate) fn apply_check_observation(app: &mut super::App, obs: &ExternalFileObservation) {
    match obs.status {
        ExternalFileStatus::NoPath => {
            app.message = Some("No file path.".to_string());
            app.pending_reload = None;
        }
        ExternalFileStatus::Unchanged => {
            app.message = Some("File unchanged on disk.".to_string());
            app.pending_reload = None;
        }
        ExternalFileStatus::Unknown(kind) => {
            app.message = Some(format!("File status check failed: {:?}", kind));
            app.pending_reload = None;
        }
        ExternalFileStatus::Modified | ExternalFileStatus::Deleted => {
            if let Some(ref p) = app.file.path {
                app.pending_reload = Some(PendingReload {
                    path: p.clone(),
                    status: obs.status.clone(),
                    snapshot: obs.live_snapshot.clone(),
                });
            } else {
                app.pending_reload = None;
            }
            let dirty = app.file.dirty;
            let text = reload_arm_message(&obs.status, dirty);
            app.message = Some(text);
        }
    }
}

/// Handle Ctrl+R for manual reload (decision + arm or perform).
/// Extracted from App::handle_key_with so mod.rs stays thin.
/// Computes one observation for the path; if matches pending exactly then
/// perform (with proper read-fail handling); else delegate to check for arm/status.
pub(crate) fn handle_reload_key(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let current_path = app.file.path.clone();
    let baseline = app.file.disk_snapshot.as_ref();
    let obs = observe_external_file(current_path.as_deref(), baseline);

    let should_perform = match (&app.pending_reload, &obs.status) {
        (Some(pend), ExternalFileStatus::Modified)
            if pend.path == current_path.clone().unwrap_or_default() =>
        {
            pend.status == ExternalFileStatus::Modified && pend.snapshot == obs.live_snapshot
        }
        (Some(pend), ExternalFileStatus::Deleted)
            if pend.path == current_path.clone().unwrap_or_default() =>
        {
            pend.status == ExternalFileStatus::Deleted && pend.snapshot == obs.live_snapshot
        }
        _ => false,
    };

    if should_perform {
        perform_observed_reload(app, &obs);
        app.render(out)?;
    } else {
        // Reuse the single observation already computed; do not re-observe.
        apply_check_observation(app, &obs);
        app.render(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "catomic_reload_policy_{}_{}",
            std::process::id(),
            name
        ));
        p
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn modified_reload_buffer_uses_editable_pages_for_huge_size() {
        let path = temp_path("huge_policy.txt");
        cleanup(&path);
        std::fs::write(&path, "first\nsecond").unwrap();

        let reloaded =
            build_modified_reload_buffer(&path, size::LARGE_FILE_LIMIT_BYTES + 1, 1).unwrap();

        assert_eq!(reloaded.size_tier, FileSizeTier::Huge);
        assert!(!reloaded.buffer.is_read_only());
        assert_eq!(reloaded.buffer.line(0).as_deref(), Some("first"));
        assert!(reloaded.buffer.page_info().unwrap().has_next);

        cleanup(&path);
    }

    #[test]
    fn modified_reload_buffer_uses_paged_buffer_for_extreme_policy() {
        let path = temp_path("extreme_policy.txt");
        cleanup(&path);
        std::fs::write(&path, "first\nsecond").unwrap();

        let reloaded =
            build_modified_reload_buffer(&path, size::HUGE_FILE_LIMIT_BYTES + 1, 1).unwrap();

        assert_eq!(reloaded.size_tier, FileSizeTier::Extreme);
        assert!(!reloaded.buffer.is_read_only());
        assert!(reloaded.buffer.page_info().unwrap().has_next);

        cleanup(&path);
    }
}
