//! Purpose: define App state and wire its focused coordination modules.
//! Owns: top-level editor state, capability-bearing services, and module boundaries.
//! Must not: implement the terminal loop, input precedence, frame composition, or buffer edits.
//! Invariants: Plain constructs no Project/network/process services; new top-level state
//!   requires an ownership review before bypassing an existing aggregate or subsystem.

use std::collections::VecDeque;
use std::io;

use crate::buffer::Buffer;
use crate::config::big_files::BigFileConfig;
use crate::config::cat::CatConfig;
use crate::config::commands::CommandConfig;
use crate::config::editor::EditorConfig;
use crate::config::keybindings::KeyBindings;
use crate::config::theme::Theme;
use crate::config::view_preferences::ViewPreferences;
use crate::file;

use crate::mode::{Capabilities, Mode};
use crate::terminal as term;

mod file_state;
pub use file_state::FileState;

#[cfg(test)]
use file_state::external_file_status;

mod autocomplete;
mod buffers;
mod command_prompt;
mod completion;
mod construction;
mod external_command;
mod external_diff;
mod help;
mod hooks;
mod indentation;
mod inline_clanker;
mod open;
mod overwrite;
mod paging;
mod project_files;
mod project_mode;
mod recovery;
mod reload;
mod render;
mod replace;
mod repo_llm;
mod runtime;
mod save;
mod search;
mod selection;
mod startup_config;
mod status;
mod surfaces;
mod undo_redo;
mod view;
mod viewport;
mod watch;

mod input;
mod lint;
mod llm_answer;
mod llm_preview;
mod llm_request;
mod mobile;
mod model_picker;
mod model_session;
mod navigation;

use startup_config::StartupConfig;

/// High-level application state for the editor.
pub struct App {
    pub mode: Mode,
    pub caps: Capabilities,
    /// Project lifetime marker/root. Strictly absent throughout Plain mode.
    pub(crate) project: Option<crate::project::ProjectSession>,
    /// Plain-safe paging policy loaded once at startup.
    pub(crate) big_files: BigFileConfig,
    /// Default-on policy for automatically reloading clean external changes.
    /// Dirty buffers always retain their explicit confirmation path.
    pub(crate) auto_reload: bool,
    /// Plain-safe editor defaults and extension-specific settings loaded at startup.
    pub(crate) editor_config: EditorConfig,
    /// Plain-safe normal-mode chord overrides; contains no command runner.
    pub(crate) keybindings: KeyBindings,
    /// Session-wide direct-typing mode. Prompts and read-only views never consume it.
    pub(crate) typing_mode: overwrite::TypingMode,
    /// Named command policy only; no process exists until explicit invocation or a hook.
    pub(crate) command_config: CommandConfig,
    /// Presentation-only cat touches; never changes editing or file semantics.
    pub(crate) cat_config: CatConfig,
    /// Semantic status colors selected once from the validated theme and terminal capability.
    pub(crate) status_theme: term::render::StatusTheme,
    /// Session-global line-number default plus its explicit persistence target.
    /// Unlike the remaining view options, this applies to every open buffer.
    pub(crate) view_preferences: ViewPreferences,
    /// Validated semantic colors loaded atomically with startup configuration.
    pub(crate) theme: Theme,
    /// Session-level opt-in autocomplete policy; contains no client at startup.
    pub(crate) autocomplete: autocomplete::AutocompleteState,
    /// Plain-safe touch UI; disabled unless Android/Termux/config explicitly enables it.
    pub(crate) mobile: mobile::MobileUiState,
    /// The active buffer (trait object for now; concrete type behind it).
    pub buffer: Box<dyn Buffer>,
    /// File path and dirty tracking.
    pub file: FileState,
    /// Gated, best-effort FileWatcher owned by App.
    /// Some only when caps.file_watch && file.path.is_some() && parent watchable.
    /// Construction failure never prevents opening or editing the file.
    /// Watcher signals are consumed only by the runtime loop via watch::check_file_watcher_once
    /// (once per iteration, as hints only). Fresh observations auto-reload clean
    /// buffers when configured; dirty buffers retain confirmation.
    /// Lifecycle is refreshed only after successful path-state changes (open/first save/Save As).
    pub file_watcher: Option<file::watcher::FileWatcher>,
    /// Whether we should exit the loop.
    pub should_quit: bool,
    /// Minimal message for user (error, quit warning, etc.). Completed messages clear on the
    /// next editor action; active surfaces own theirs, and confirmations survive only the
    /// matching action.
    pub message: Option<String>,
    /// When true, an immediately following quit action while dirty will force quit (no save).
    pub pending_quit_confirm: bool,
    /// When Some, records a token bound to the concrete observed disk state
    /// (path + ExternalFileStatus + live FileSnapshot) at the time of a first
    /// Ctrl+S refusal. Second Ctrl+S forces only if a fresh observation matches
    /// the token (for Modified: identical snapshot; Deleted/Unknown by kind).
    /// Cleared on content edits, successful save, and path changes.
    /// Any unrelated editor action cancels it; resize/render must not touch it.
    pub pending_save_conflict: Option<save::PendingSaveConflict>,
    /// Pending reload confirmation (Phase 2-s). Armed by first Ctrl+R on Modified/Deleted
    /// when status indicates disk differs. Second Ctrl+R reloads only on exact snapshot match.
    /// Cleared by content edits (insert/delete/undo/redo), successful save, path changes.
    /// Any unrelated editor action cancels it; resize/render do not clear.
    /// NoPath/Unchanged/Unknown do not arm.
    pub pending_reload: Option<reload::PendingReload>,
    /// Explicit Ctrl+F prompt/worker state. No worker exists before invocation.
    pub(crate) search: search::SearchUiState,
    /// Explicit two-stage replace prompt; empty outside direct user invocation.
    pub(crate) replace: replace::ReplaceState,
    /// Global transient goto/command prompt. It constructs no background service.
    pub(crate) command_prompt: command_prompt::CommandPromptState,
    /// Plain-safe local completion UI, constructed only when its capability is enabled.
    pub(crate) completion: Option<completion::CompletionUiState>,
    /// Transient read-only surfaces. New surfaces require an ownership review before
    /// adding another top-level App field; clients/workers never belong in this group.
    pub(crate) surfaces: surfaces::SurfaceState,
    /// Local confirmation state only; contains bounded context/settings but no HTTP client.
    pub(crate) pending_llm_request: Option<llm_request::PendingLlmRequest>,
    /// Present only after explicit Enter confirmation; dropping it cancels the transient client.
    pub(crate) llm_task: Option<llm_request::RunningLlmRequest>,
    /// Process-local model override shared across buffers and never persisted implicitly.
    pub(crate) model_session: model_session::ModelSession,
    /// Explicit searchable picker and bounded discovery cache; idle and network-free by default.
    pub(crate) model_picker: model_picker::ModelPickerState,
    /// One-key inline clanker phase. No client exists in warning/confirmation state.
    pub(crate) inline_clanker: inline_clanker::InlineClankerState,
    /// Per-buffer render-only history for the latest accepted inline-clanker changes.
    pub(crate) clanker_changes: inline_clanker::ChangeHistory,
    /// Per-buffer render-only metadata for the exact latest external reload revision.
    pub(crate) external_changes: external_diff::ExternalChanges,
    /// Project-only repo-context preparation, confirmation, or confirmed network task.
    pub(crate) repo_llm_state: Option<repo_llm::RepoLlmState>,
    /// External process/preview state; empty at startup and while unused.
    pub(crate) external_command: external_command::ExternalCommandState,
    /// Lifecycle command queue; contains no process and is empty without configured events.
    pub(crate) hooks: hooks::HookState,
    /// Opt-in per-buffer catnap timer/task/preview state; idle and write-free by default.
    pub(crate) recovery: recovery::RecoveryState,
    /// Per-buffer half-open selection state.
    pub(crate) selection: selection::SelectionUiState,
    /// Always-available process-local clipboard shared across open buffers.
    pub(crate) clipboard: String,
    /// Consecutive cut-line commands append to the session clipboard until another action.
    pub(crate) cut_line_append: bool,
    /// Remaining per-buffer display toggles; they never mutate document content.
    pub(crate) view: view::ViewOptions,
    /// Inactive buffers in next-buffer order. The active buffer remains in the
    /// established App fields so editing/render paths stay direct and boring.
    pub(crate) inactive_buffers: VecDeque<buffers::BufferSlot>,
    /// Zero-based position of the active buffer in the logical buffer ring.
    pub(crate) active_buffer_index: usize,
    /// Terminal screen size and scroll state. Single source of truth for render height.
    /// Initialized conservatively; updated from crossterm after setup and on resize.
    pub screen: term::screen::Screen,
}

impl App {
    /// Reveal the current cursor row/col so they are visible in the content area.
    /// Called after cursor movement and content mutations (insert, delete, undo/redo).
    /// Clamps first for zero-size terminals so reveal_* see a sane starting point.
    pub(crate) fn reveal_cursor(&mut self) {
        viewport::reveal_cursor(self)
    }

    /// Returns whether (and how) the on-disk file differs from our last captured snapshot.
    /// Used by future watch/reload to decide action; for 2-l this is detection only.
    /// Must not mutate buffer, file state (dirty/snapshot), message, pending, viewport, or history.
    /// NoPath for untitled; delegates to the bounded file-state identity helper.
    #[cfg(test)]
    fn external_file_status(&self) -> crate::file::io::ExternalFileStatus {
        external_file_status(&self.file)
    }
}

/// Public entry called from main.rs.
pub fn run(initial_file: Option<&str>) -> io::Result<()> {
    let config = StartupConfig::load()?;
    let mut app = App::new_with_config(initial_file, config)?;
    app.run()
}

/// Open the resolved configuration in Catomic without requiring that configuration
/// to parse successfully first. Missing-file creation stays inside the live terminal
/// session so a terminal setup failure cannot leave a newly created file behind.
pub fn run_config() -> io::Result<()> {
    let path = crate::config::user_file::path()?;
    let file = path
        .to_str()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "config path is not valid UTF-8")
        })?
        .to_string();
    let config = StartupConfig::without_user_config()?;
    let mut app = App::new_with_config(Some(&file), config)?;
    app.run_config(path)
}

#[cfg(test)]
mod tests;
