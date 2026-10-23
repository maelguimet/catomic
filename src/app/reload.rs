//! Manual reload-from-disk confirmation.
//!
//! Purpose: record unresolved disk changes, arm `Confirmation::Reload`, and perform reloads.
//! Owns: ObservedExternalChange, reload messages, arm/perform helpers, handle_reload_key.
//! Must not: own watcher polling, background work, snapshot capture policy,
//!   config parsing, repository scans, or external services.
//! Invariants: an armed reload is bound to path + exact modified snapshot or deletion;
//!   watcher observations never arm it; unrelated actions disarm it while keeping the
//!   observed change; a second explicit press only acts on an exact match;
//!   automatic reload is invoked only for clean buffers by caller policy;
//!   successful reloads refresh watcher path identities.

use std::io;
use std::path::{Path, PathBuf};

use super::confirmation::{ArmedReload, Confirmation, ConflictingChange, ReloadableChange};
use crate::buffer;
use crate::file::io::{
    observe_external_file, ExternalFileObservation, ExternalFileStatus, FileSnapshot,
};
use crate::file::size::{self, FileSizeTier, OpenSizeDecision};

/// Unresolved disk change for the active buffer, kept for display only.
/// Arming a reload is a separate `Confirmation::Reload` bound to the same change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObservedExternalChange {
    pub path: PathBuf,
    pub change: ConflictingChange,
}

pub(crate) fn forget_external_observation(app: &mut super::App) {
    app.observed_change = None;
    app.confirmation.disarm_reload();
}

/// Retain a fresh unresolved disk observation without treating it as a user
/// confirmation. Modified, Deleted, and Unknown observations all mean the
/// accepted buffer/disk relationship cannot currently be called saved.
pub(crate) fn remember_external_observation(app: &mut super::App, obs: &ExternalFileObservation) {
    app.confirmation.disarm_reload();
    app.observed_change = app
        .file
        .path
        .clone()
        .zip(ConflictingChange::observed(obs))
        .map(|(path, change)| ObservedExternalChange { path, change });
}

/// Returns the message for first Ctrl+R press that arms a reload confirmation.
pub(crate) fn reload_arm_message(change: &ReloadableChange, dirty: bool) -> String {
    match change {
        ReloadableChange::Modified(_) => {
            if dirty {
                "File changed on disk. Press Ctrl+R again to reload from disk (discard local changes).".to_string()
            } else {
                "File changed on disk. Press Ctrl+R again to reload from disk.".to_string()
            }
        }
        ReloadableChange::Deleted => {
            if dirty {
                "File deleted on disk. Press Ctrl+R again to clear buffer (discard local changes)."
                    .to_string()
            } else {
                "File deleted on disk. Press Ctrl+R again to clear buffer.".to_string()
            }
        }
    }
}

pub(crate) fn reload_arm_message_for_ui(
    change: &ReloadableChange,
    dirty: bool,
    mobile: bool,
) -> String {
    if !mobile {
        return reload_arm_message(change, dirty);
    }
    match change {
        ReloadableChange::Modified(_) => mobile_reload_message(
            "File changed on disk. Tap Menu > Check / reload file again to reload from disk",
            dirty,
        ),
        ReloadableChange::Deleted => mobile_reload_message(
            "File deleted on disk. Tap Menu > Check / reload file again to clear the buffer",
            dirty,
        ),
    }
}

/// Returns the warning for a passive watcher observation. Unlike the explicit
/// arm message, this tells the user that two actions are still required.
pub(crate) fn reload_watch_message_for_ui(
    change: &ReloadableChange,
    dirty: bool,
    mobile: bool,
) -> String {
    if mobile {
        return match change {
            ReloadableChange::Modified(_) => mobile_reload_message(
                "File changed on disk. Tap Menu > Check / reload file twice to reload from disk",
                dirty,
            ),
            ReloadableChange::Deleted => mobile_reload_message(
                "File deleted on disk. Tap Menu > Check / reload file twice to clear the buffer",
                dirty,
            ),
        };
    }
    match change {
        ReloadableChange::Modified(_) => {
            if dirty {
                "File changed on disk. Press Ctrl+R twice to reload from disk (discard local changes)."
                    .to_string()
            } else {
                "File changed on disk. Press Ctrl+R twice to reload from disk.".to_string()
            }
        }
        ReloadableChange::Deleted => {
            if dirty {
                "File deleted on disk. Press Ctrl+R twice to clear buffer (discard local changes)."
                    .to_string()
            } else {
                "File deleted on disk. Press Ctrl+R twice to clear buffer.".to_string()
            }
        }
    }
}

pub(crate) fn reload_drift_message(change: &ReloadableChange, dirty: bool) -> String {
    let local = if dirty {
        " Local changes preserved."
    } else {
        ""
    };
    match change {
        ReloadableChange::Modified(_) => format!(
            "File changed again on disk. Press Ctrl+R to re-arm reload confirmation.{local}"
        ),
        ReloadableChange::Deleted => format!(
            "File was deleted after reload was armed. Press Ctrl+R to re-arm confirmation.{local}"
        ),
    }
}

pub(crate) fn reload_drift_message_for_ui(
    change: &ReloadableChange,
    dirty: bool,
    mobile: bool,
) -> String {
    if !mobile {
        return reload_drift_message(change, dirty);
    }
    let local = if dirty {
        " Local changes preserved."
    } else {
        ""
    };
    match change {
        ReloadableChange::Modified(_) => format!(
            "File changed again on disk. Tap Menu > Check / reload file to re-arm confirmation.{local}"
        ),
        ReloadableChange::Deleted => format!(
            "File was deleted after reload was armed. Tap Menu > Check / reload file to re-arm confirmation.{local}"
        ),
    }
}

fn mobile_reload_message(prefix: &str, dirty: bool) -> String {
    if dirty {
        format!("{prefix} and discard local changes.")
    } else {
        format!("{prefix}.")
    }
}

struct ReloadedModifiedBuffer {
    buffer: Box<dyn buffer::Buffer>,
    snapshot: FileSnapshot,
    size_bytes: u64,
    size_tier: FileSizeTier,
    text_format: crate::file::text_format::TextFormat,
}

fn observed_present_snapshot(obs: &ExternalFileObservation) -> io::Result<&FileSnapshot> {
    match obs.live_snapshot.as_ref() {
        Some(snapshot @ FileSnapshot::Present { .. }) => Ok(snapshot),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reload modified path missing present size snapshot",
        )),
    }
}

fn paged_reload_buffer(
    path: &Path,
    mut source: crate::file::io::PinnedFile,
    page_lines: usize,
) -> io::Result<(
    Box<dyn buffer::Buffer>,
    crate::file::text_format::TextFormat,
)> {
    let loaded_snapshot = source.snapshot().clone();
    let format = crate::file::text_format::detect_file_format_from(source.file_mut())?;
    if format.utf8_bom || format.line_ending == crate::file::text_format::LineEnding::Cr {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UTF-8 BOM and CR-only files must be opened below the paged-file threshold",
        ));
    }
    source.ensure_descriptor_unchanged(path)?;
    let buffer = buffer::PagedFileBuffer::from_file(source.into_file(), page_lines)?;
    crate::file::io::ensure_path_matches_snapshot(path, &loaded_snapshot)?;
    Ok((Box::new(buffer), format))
}

fn build_modified_reload_buffer(
    path: &Path,
    expected: &FileSnapshot,
    page_lines: usize,
) -> io::Result<ReloadedModifiedBuffer> {
    let mut source = crate::file::io::PinnedFile::open(path)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Interrupted,
            format!("file disappeared while reloading: {}", path.display()),
        )
    })?;
    if source.snapshot() != expected {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("file changed after reload confirmation: {}", path.display()),
        ));
    }
    let FileSnapshot::Present {
        len: size_bytes, ..
    } = source.snapshot()
    else {
        unreachable!("PinnedFile always captures a present regular file")
    };
    let size_bytes = *size_bytes;
    let loaded_snapshot = source.snapshot().clone();
    let size_tier = size::classify_file_size(size_bytes);
    let (buffer, text_format): (
        Box<dyn buffer::Buffer>,
        crate::file::text_format::TextFormat,
    ) = match size::open_size_decision(size_bytes) {
        OpenSizeDecision::Paged => paged_reload_buffer(path, source, page_lines)?,
        OpenSizeDecision::Normal | OpenSizeDecision::Warn => {
            let bytes = source.read_all_verified(path)?;
            let decoded = crate::file::text_format::decode(bytes)?;
            (
                Box::new(buffer::PieceTable::from_owned_text(decoded.text)),
                decoded.format,
            )
        }
    };

    Ok(ReloadedModifiedBuffer {
        buffer,
        snapshot: loaded_snapshot,
        size_bytes,
        size_tier,
        text_format,
    })
}

fn reload_modified_warning(size_bytes: u64, size_tier: FileSizeTier) -> Option<String> {
    if matches!(size_tier, FileSizeTier::Huge | FileSizeTier::Extreme) {
        if let Some(warning) = size::open_size_warning_message(size_bytes, size_tier) {
            return Some(warning);
        }
    }
    None
}

/// Replace a clean buffer from one already-fresh Modified/Deleted observation.
/// Watcher policy and Ctrl+R confirmation both call this narrow mutation seam.
/// Errors are surfaced in `message` and leave the existing buffer intact.
pub(crate) fn perform_observed_reload(app: &mut super::App, obs: &ExternalFileObservation) {
    let Some(path) = app.file.path.clone() else {
        app.message_info("No file path.");
        return;
    };
    remember_external_observation(app, obs);
    match obs.status {
        ExternalFileStatus::Modified => {
            match observed_present_snapshot(obs).and_then(|expected| {
                build_modified_reload_buffer(&path, expected, app.big_files.page_lines)
            }) {
                Ok(reloaded) => {
                    if let Err(error) = apply_modified_reload(app, &path, reloaded) {
                        report_reload_error(app, error);
                    }
                }
                Err(error) => report_reload_error(app, error),
            }
        }
        ExternalFileStatus::Deleted => {
            match crate::file::io::ensure_path_matches_snapshot(&path, &FileSnapshot::Absent) {
                Ok(()) => apply_deleted_reload(app),
                Err(error) => report_reload_error(app, error),
            }
        }
        _ => apply_check_observation(app, obs),
    }
}

fn apply_modified_reload(
    app: &mut super::App,
    path: &Path,
    reloaded: ReloadedModifiedBuffer,
) -> io::Result<()> {
    let external_diff = super::external_diff::compare(&*app.buffer, &*reloaded.buffer);
    crate::file::io::ensure_path_matches_snapshot(path, &reloaded.snapshot)?;
    let reload_warning = match (
        reload_modified_warning(reloaded.size_bytes, reloaded.size_tier),
        external_diff_warning(&external_diff),
    ) {
        (Some(size), Some(diff)) => Some(format!("{size} {diff}")),
        (size, diff) => size.or(diff),
    };
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
    super::lint::invalidate(app);
    super::view::cancel_preview(app);
    app.selection.clear();
    app.buffer = reloaded.buffer;
    super::file_state::note_content_change(&mut app.file);
    app.external_changes = external_diff.into_changes();
    app.file.saved_history_position = app.buffer.edit_history_position();
    app.file.saved_history_unavailable = false;
    app.file.dirty = false;
    app.file.text_format = reloaded.text_format;
    app.file.disk_snapshot = Some(reloaded.snapshot);
    app.file.size_bytes = Some(reloaded.size_bytes);
    app.file.size_tier = Some(reloaded.size_tier);
    finish_reload(app, reload_warning);
    Ok(())
}

fn apply_deleted_reload(app: &mut super::App) {
    let cleared: Box<dyn buffer::Buffer> = Box::new(buffer::PieceTable::new());
    let external_diff = super::external_diff::compare(&*app.buffer, &*cleared);
    let reload_warning = external_diff_warning(&external_diff);
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
    super::lint::invalidate(app);
    super::view::cancel_preview(app);
    app.selection.clear();
    app.buffer = cleared;
    super::file_state::note_content_change(&mut app.file);
    app.external_changes = external_diff.into_changes();
    app.file.saved_history_position = app.buffer.edit_history_position();
    app.file.saved_history_unavailable = false;
    app.file.dirty = false;
    app.file.disk_snapshot = Some(FileSnapshot::Absent);
    app.file.size_bytes = None;
    app.file.size_tier = None;
    finish_reload(app, reload_warning);
}

fn external_diff_warning(outcome: &super::external_diff::DiffOutcome) -> Option<String> {
    match outcome {
        super::external_diff::DiffOutcome::Skipped(reason) => {
            Some(format!("External change highlighting skipped: {reason}."))
        }
        super::external_diff::DiffOutcome::Compared(_) => None,
    }
}

fn report_reload_error(app: &mut super::App, error: io::Error) {
    if error.kind() == io::ErrorKind::Interrupted {
        let local = if app.file.dirty {
            " Local changes preserved."
        } else {
            ""
        };
        app.message_warning(format!(
            "Reload aborted because the file changed again. Re-arm reload confirmation.{local}"
        ));
    } else {
        app.message_error(format!("Reload error: {error}"));
    }
}

fn finish_reload(app: &mut super::App, message: Option<String>) {
    super::watch::refresh_file_watcher(app);
    app.message = message;
    app.message_role = crate::terminal::render::StatusRole::Info;
    app.observed_change = None;
    app.confirmation = Confirmation::None;
    app.reveal_cursor();
}

/// Apply a single ExternalFileObservation from an explicit reload action.
/// This is the single-source status+arm path for manual checks.
/// NoPath/Unknown report a problem; Unchanged restores normal status; all clear pending.
/// Modified/Deleted: arm pending bound to obs.live_snapshot (for drift), set arm message.
/// Does not mutate buffer, dirty, disk_snapshot, or history.
pub(crate) fn apply_check_observation(app: &mut super::App, obs: &ExternalFileObservation) {
    apply_observation(app, obs, true);
}

/// Record a passive watcher observation without arming destructive reload.
pub(crate) fn apply_watch_observation(app: &mut super::App, obs: &ExternalFileObservation) {
    apply_observation(app, obs, false);
}

fn apply_observation(
    app: &mut super::App,
    obs: &ExternalFileObservation,
    is_explicit_action: bool,
) {
    let keeps_arm = armed_reload_matches(app, obs);
    remember_external_observation(app, obs);
    let Some(observed) = app.observed_change.clone() else {
        if obs.status == ExternalFileStatus::NoPath {
            app.message_info("No file path.");
        } else {
            app.message = None;
        }
        return;
    };
    let change = match observed.change {
        ConflictingChange::Unknown(kind) => {
            app.message_error(format!("File status check failed: {:?}", kind));
            return;
        }
        ConflictingChange::Modified(snapshot) => ReloadableChange::Modified(snapshot),
        ConflictingChange::Deleted => ReloadableChange::Deleted,
    };
    let dirty = app.file.dirty;
    let mobile = super::mobile::is_enabled(app);
    let text = if is_explicit_action || keeps_arm {
        let text = reload_arm_message_for_ui(&change, dirty, mobile);
        app.confirmation = Confirmation::Reload(ArmedReload {
            path: observed.path,
            change,
        });
        text
    } else {
        reload_watch_message_for_ui(&change, dirty, mobile)
    };
    app.message_warning(text);
}

/// Handle Ctrl+R for manual reload (decision + arm or perform).
/// Extracted from App::handle_key_with so mod.rs stays thin.
/// Computes one observation for the path; if matches pending exactly then
/// perform (with proper read-fail handling); else delegate to check for arm/status.
pub(crate) fn handle_reload_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    let current_path = app.file.path.clone();
    let baseline = app.file.disk_snapshot.as_ref();
    let obs = observe_external_file(current_path.as_deref(), baseline);

    if armed_reload_matches(app, &obs) {
        perform_observed_reload(app, &obs);
        app.render(out)?;
    } else {
        apply_check_observation(app, &obs);
        app.render(out)?;
    }
    Ok(())
}

pub(crate) fn armed_reload_matches(app: &super::App, obs: &ExternalFileObservation) -> bool {
    let (Some(armed), Some(path)) = (app.confirmation.armed_reload(), &app.file.path) else {
        return false;
    };
    armed.path == *path
        && ConflictingChange::observed(obs)
            .and_then(|change| change.reloadable())
            .is_some_and(|change| change == armed.change)
}

#[cfg(test)]
#[path = "reload/paged_tests.rs"]
mod paged_tests;

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
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(size::LARGE_FILE_LIMIT_BYTES + 1)
            .unwrap();
        let expected = crate::file::io::capture_file_snapshot(&path).unwrap();

        let reloaded = build_modified_reload_buffer(&path, &expected, 1).unwrap();

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
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(size::HUGE_FILE_LIMIT_BYTES + 1)
            .unwrap();
        let expected = crate::file::io::capture_file_snapshot(&path).unwrap();

        let reloaded = build_modified_reload_buffer(&path, &expected, 1).unwrap();

        assert_eq!(reloaded.size_tier, FileSizeTier::Extreme);
        assert!(!reloaded.buffer.is_read_only());
        assert!(reloaded.buffer.page_info().unwrap().has_next);

        cleanup(&path);
    }

    #[test]
    fn huge_reload_stays_paged_and_explicitly_skips_external_highlighting() {
        use std::io::{Seek, SeekFrom, Write};

        let path = temp_path("huge_external_diff_skip.txt");
        cleanup(&path);
        std::fs::write(&path, "before\nsecond").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(size::LARGE_FILE_LIMIT_BYTES + 1)
            .unwrap();
        let mut app = super::super::App::new(Some(&path.to_string_lossy())).unwrap();

        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"AFTER!").unwrap();
        file.sync_all().unwrap();
        let observation = observe_external_file(Some(&path), app.file.disk_snapshot.as_ref());
        assert_eq!(observation.status, ExternalFileStatus::Modified);

        perform_observed_reload(&mut app, &observation);

        assert_eq!(app.buffer.line(0).as_deref(), Some("AFTER!"));
        assert!(app.buffer.page_info().is_some());
        assert!(app
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("External change highlighting skipped"));
        assert!(app
            .external_changes
            .visible(app.buffer.content_revision())
            .is_none());
        cleanup(&path);
    }

    #[test]
    fn loaded_revision_cannot_adopt_a_later_path_revision_as_baseline() {
        let path = temp_path("loaded_b_path_c.txt");
        cleanup(&path);
        std::fs::write(&path, "base").unwrap();
        let mut app = super::super::App::new(Some(&path.to_string_lossy())).unwrap();
        app.buffer.insert_char('L');
        app.file.dirty = true;
        let local_buffer = app.buffer.to_string();
        let base_snapshot = app.file.disk_snapshot.clone();

        std::fs::write(&path, "BBBB").unwrap();
        let observation = observe_external_file(Some(&path), app.file.disk_snapshot.as_ref());
        assert_eq!(observation.status, ExternalFileStatus::Modified);
        let expected = observed_present_snapshot(&observation).unwrap();
        let reloaded = build_modified_reload_buffer(&path, expected, 20_000).unwrap();
        assert_eq!(reloaded.buffer.to_string(), "BBBB");

        std::fs::write(&path, "CCCC").unwrap();
        let error = apply_modified_reload(&mut app, &path, reloaded)
            .expect_err("path revision C must not baseline loaded revision B");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(app.buffer.to_string(), local_buffer);
        assert!(app.file.dirty);
        assert_eq!(app.file.disk_snapshot, base_snapshot);
        assert_eq!(
            observe_external_file(Some(&path), app.file.disk_snapshot.as_ref()).status,
            ExternalFileStatus::Modified
        );
        cleanup(&path);
    }

    #[test]
    fn confirmed_revision_drift_requires_rearming_and_preserves_local_edits() {
        let path = temp_path("confirmed_b_loaded_c.txt");
        cleanup(&path);
        std::fs::write(&path, "base").unwrap();
        let mut app = super::super::App::new(Some(&path.to_string_lossy())).unwrap();
        app.buffer.insert_char('L');
        app.file.dirty = true;
        let local_buffer = app.buffer.to_string();
        let base_snapshot = app.file.disk_snapshot.clone();

        std::fs::write(&path, "BBBB").unwrap();
        let confirmed = observe_external_file(Some(&path), app.file.disk_snapshot.as_ref());
        apply_check_observation(&mut app, &confirmed);
        assert!(app.observed_change.is_some());

        std::fs::write(&path, "CCCC").unwrap();
        perform_observed_reload(&mut app, &confirmed);

        assert_eq!(app.buffer.to_string(), local_buffer);
        assert!(app.file.dirty);
        assert_eq!(app.file.disk_snapshot, base_snapshot);
        assert!(app.observed_change.is_some() && app.confirmation.armed_reload().is_none());
        assert!(app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Re-arm reload confirmation"));
        cleanup(&path);
    }

    #[test]
    fn clean_revision_drift_retains_not_saved_observation() {
        let path = temp_path("clean_confirmed_b_loaded_c.txt");
        cleanup(&path);
        std::fs::write(&path, "base").unwrap();
        let mut app = super::super::App::new(Some(&path.to_string_lossy())).unwrap();
        let base_snapshot = app.file.disk_snapshot.clone();

        std::fs::write(&path, "BBBB").unwrap();
        let confirmed = observe_external_file(Some(&path), app.file.disk_snapshot.as_ref());
        apply_check_observation(&mut app, &confirmed);
        assert!(app.confirmation.armed_reload().is_some());

        std::fs::write(&path, "CCCC").unwrap();
        perform_observed_reload(&mut app, &confirmed);

        assert_eq!(app.buffer.to_string(), "base");
        assert!(!app.file.dirty);
        assert_eq!(app.file.disk_snapshot, base_snapshot);
        assert!(app.observed_change.is_some() && app.confirmation.armed_reload().is_none());
        assert!(super::super::render::status_line(&app)
            .text
            .contains("(not saved)"));
        cleanup(&path);
    }
}
