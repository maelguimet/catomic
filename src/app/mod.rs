//! App state + the one blessed goblin loop.
//!
//! Per TODO.md:
//! - "Keep the main (goblin) loop extremely boring and in one obvious place."
//! - Phase 0: ultra-minimal MVP. Cursor, insert, delete, open, save, quit.
//! - Buffer trait lives in `buffer`.
//!
//! This module owns high-level state (current buffer, mode, capabilities,
//! terminal handle, etc.) and the event loop.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyEvent};

use crate::buffer::Buffer;
use crate::config::big_files::BigFileConfig;
use crate::config::commands::CommandConfig;
use crate::config::editor::EditorConfig;
use crate::config::keybindings::KeyBindings;
use crate::file;

use crate::mode::{Capabilities, Mode};
use crate::terminal as term;

mod file_state;
pub use file_state::FileState;

#[cfg(test)]
use file_state::external_file_status;

mod buffers;
mod command_prompt;
mod completion;
mod external_command;
mod hooks;
mod open;
mod paging;
mod project_files;
mod project_mode;
mod reload;
mod repo_llm;
mod save;
mod search;
mod selection;
mod status;
mod view;
mod viewport;
mod watch;

mod input;
mod lint;
mod llm_answer;
mod llm_preview;
mod llm_request;

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
    /// Named command policy only; no process exists until explicit invocation or a hook.
    pub(crate) command_config: CommandConfig,
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
    /// Lifecycle is refreshed only after successful path-state changes (new + save-none->path).
    pub file_watcher: Option<file::watcher::FileWatcher>,
    /// Whether we should exit the loop.
    pub should_quit: bool,
    /// Minimal message for user (error, quit warning, etc.). Cleared on edits or explicit.
    pub message: Option<String>,
    /// When true, a second Ctrl+Q while dirty will force quit (no save).
    pub pending_quit_confirm: bool,
    /// When Some, records a token bound to the concrete observed disk state
    /// (path + ExternalFileStatus + live FileSnapshot) at the time of a first
    /// Ctrl+S refusal. Second Ctrl+S forces only if a fresh observation matches
    /// the token (for Modified: identical snapshot; Deleted/Unknown by kind).
    /// Cleared on content edits, successful save, and path changes.
    /// Movement/resize/render must not touch it.
    pub pending_save_conflict: Option<save::PendingSaveConflict>,
    /// Pending reload confirmation (Phase 2-s). Armed by first Ctrl+R on Modified/Deleted
    /// when status indicates disk differs. Second Ctrl+R reloads only on exact snapshot match.
    /// Cleared by content edits (insert/delete/undo/redo), successful save, path changes.
    /// Movement/resize/render do not clear. NoPath/Unchanged/Unknown do not arm.
    pub pending_reload: Option<reload::PendingReload>,
    /// Explicit Ctrl+F prompt/worker state. No worker exists before invocation.
    pub(crate) search: search::SearchUiState,
    /// Global transient goto/command prompt. It constructs no background service.
    pub(crate) command_prompt: command_prompt::CommandPromptState,
    /// Plain-safe local completion UI, constructed only when its capability is enabled.
    pub(crate) completion: Option<completion::CompletionUiState>,
    /// Project-only read-only diagnostics document, absent until explicitly shown.
    pub(crate) lint_view: Option<lint::DiagnosticsView>,
    /// Project-only read-only discovered-file picker, absent until explicitly shown.
    pub(crate) project_files_view: Option<project_files::ProjectFilesView>,
    /// Explicit LLM patch preview, absent until a proposal has been parsed and validated.
    /// This contains no network client; repo previews retain their immutable drift guard.
    pub(crate) llm_preview: Option<llm_preview::PatchPreview>,
    /// Explicit read-only model explanation; contains output text and no client.
    pub(crate) llm_answer: Option<llm_answer::AnswerView>,
    /// Local confirmation state only; contains bounded context/settings but no HTTP client.
    pub(crate) pending_llm_request: Option<llm_request::PendingLlmRequest>,
    /// Present only after explicit Enter confirmation; dropping it cancels the transient client.
    pub(crate) llm_task: Option<llm_request::RunningLlmRequest>,
    /// Project-only repo-context preparation, confirmation, or confirmed network task.
    pub(crate) repo_llm_state: Option<repo_llm::RepoLlmState>,
    /// External process/preview state; empty at startup and while unused.
    pub(crate) external_command: external_command::ExternalCommandState,
    /// Lifecycle command queue; contains no process and is empty without configured events.
    pub(crate) hooks: hooks::HookState,
    /// Per-buffer half-open selection state.
    pub(crate) selection: selection::SelectionUiState,
    /// Always-available process-local clipboard shared across open buffers.
    pub(crate) clipboard: String,
    /// Per-buffer display toggles; they never mutate document content.
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
    #[cfg(test)]
    pub fn new(initial_path: Option<&str>) -> io::Result<Self> {
        Self::new_with_big_file_config(initial_path, BigFileConfig::default())
    }

    #[cfg(test)]
    pub(crate) fn new_with_big_file_config(
        initial_path: Option<&str>,
        big_files: BigFileConfig,
    ) -> io::Result<Self> {
        Self::new_with_config(
            initial_path,
            big_files,
            true,
            EditorConfig::default(),
            KeyBindings::default(),
            CommandConfig::default(),
        )
    }

    pub(crate) fn new_with_config(
        initial_path: Option<&str>,
        big_files: BigFileConfig,
        auto_reload: bool,
        editor_config: EditorConfig,
        keybindings: KeyBindings,
        command_config: CommandConfig,
    ) -> io::Result<Self> {
        let mode = Mode::Plain; // Start in Plain by default. User can switch later.
        let caps = Capabilities::from_mode(mode);
        let completion = caps
            .local_completion
            .then(completion::CompletionUiState::default);

        // Size/guardrail + initial snapshot/open plan extracted (see open.rs).
        // Single capture_file_snapshot in prepare supplies both size decision
        // and disk_snapshot/content plan (no duplicate metadata probe in the happy path).
        // Covers all App::new cases (None/missing/Small/Large/Huge/Extreme/
        // hard-meta/invalid-utf8-after-small-probe).
        let meta = open::prepare_open_file_meta(initial_path)?;

        let buffer = open::build_open_buffer(&meta, initial_path, big_files.page_lines)?;

        // Capture initial history position as the clean save point (open or new).
        let initial_pos = buffer.edit_history_position();
        // Use the single initial disk snapshot captured inside prepare_open_file_meta.
        // No second metadata probe here. prepare already returned Err for hard meta
        // errors before we reach content reads. Snapshot behavior:
        // - None path: snapshot=None
        // - missing: snapshot=Some(Absent)
        // - present: snapshot=Some(Present) from the same metadata probe used for size
        let disk_snapshot = meta.disk_snapshot;
        // Size derived in prepare from the same snapshot (None for missing/none-path).
        // See also save.rs for the narrow post-write len fallback contract.
        // Build base App first, then attach watcher via best-effort helper.
        // This keeps watcher construction failure non-fatal and avoids
        // partial-construction gymnastics in the Result path.
        let mut app = App {
            mode,
            caps,
            project: None,
            big_files,
            auto_reload,
            editor_config,
            keybindings,
            command_config,
            buffer,
            file: FileState {
                path: initial_path.map(PathBuf::from),
                dirty: false,
                saved_history_position: initial_pos,
                disk_snapshot,
                size_bytes: meta.size_bytes,
                size_tier: meta.size_tier,
            },
            file_watcher: None,
            should_quit: false,
            message: meta.initial_message,
            pending_quit_confirm: false,
            pending_save_conflict: None,
            pending_reload: None,
            search: search::SearchUiState::default(),
            command_prompt: command_prompt::CommandPromptState::default(),
            completion,
            lint_view: None,
            project_files_view: None,
            llm_preview: None,
            llm_answer: None,
            pending_llm_request: None,
            llm_task: None,
            repo_llm_state: None,
            external_command: external_command::ExternalCommandState::default(),
            hooks: hooks::HookState::default(),
            selection: selection::SelectionUiState::default(),
            clipboard: String::new(),
            view: view::ViewOptions::default(),
            inactive_buffers: VecDeque::new(),
            active_buffer_index: 0,
            // Conservative default matching prior hardcoded 24; no real term required for unit tests.
            screen: term::screen::Screen::new(80, 24),
        };
        watch::refresh_file_watcher(&mut app);
        Ok(app)
    }

    /// The main goblin loop. Keep it obvious.
    pub fn run(&mut self) -> io::Result<()> {
        // Terminal setup is in the terminal module.
        let mut stdout = io::stdout();

        // Guard *before* any mutation. Its Drop guarantees best-effort
        // teardown even on error paths after this point or partial setup.
        // Do not trust only the happy-path explicit teardown.
        let _guard = term::TerminalGuard::new();

        term::setup(&mut stdout)?;

        // Read actual terminal size using crossterm; keep conservative default on failure
        // (e.g. non-tty or test envs). Linux-first/simple: no extra handling.
        if let Ok((w, h)) = crossterm::terminal::size() {
            self.screen.update_size(w, h);
        }

        // Install panic hook to do best-effort restore even before unwind reaches guard.
        // The guard restores the previously installed hook on normal exit.
        let _panic_guard = term::PanicRestoreGuard::install();

        hooks::trigger_open(self);

        // Phase 0 render is extremely dumb.
        self.render(&mut stdout)?;

        while !self.should_quit {
            // Check watcher once per iteration (hint only). Uses the non-blocking seam
            // so we do not block the 100ms poll cycle. If a Changed/Deleted signal is
            // present, check_file_watcher_once does one try_recv + fresh observe_external_file
            // + policy application (auto-reload clean buffers or arm confirmation).
            // Render only if handled.
            // Do not call try_recv directly; only through the helper.
            // Must not be called from handle_key/save/reload/render.
            // If both signal + key arrive in one iteration: simple loop may render twice; acceptable.
            if watch::check_file_watcher_once_and_render(self, &mut stdout)? {
                // message/pending updated; render already emitted by helper
            }

            search::poll_search(self, &mut stdout)?;
            command_prompt::poll_goto(self, &mut stdout)?;
            lint::poll(self, &mut stdout)?;
            project_files::poll(self, &mut stdout)?;
            llm_request::poll(self, &mut stdout)?;
            repo_llm::poll(self, &mut stdout)?;
            external_command::poll(self, &mut stdout)?;
            hooks::pump(self, &mut stdout)?;

            // Blocking read for Phase 0. Later we may need non-blocking + resize.
            if event::poll(std::time::Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) => {
                        self.handle_key(key)?;
                    }
                    Event::Paste(text) => {
                        input::handle_paste(self, &mut stdout, &text)?;
                    }
                    Event::Mouse(mouse) => {
                        selection::handle_mouse(self, &mut stdout, mouse)?;
                    }
                    Event::Resize(w, h) => {
                        // Update screen size, reveal cursor (vert/horiz if implemented this pass),
                        // render immediately. No debounce/smart viewport.
                        self.handle_resize(w, h, &mut stdout)?;
                    }
                    _ => {}
                }
            }

            // In a real tight loop we would only render on dirty.
            // For Phase 0 we re-render after every interesting key.
            // (We do the render inside handle_key for now.)
        }

        // Explicit is still fine (idempotent), but guard Drop is the safety net.
        term::teardown(&mut stdout)?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        input::handle_key(self, key)
    }

    /// Route key handling + associated renders through a writer.
    /// Smallest seam so tests can capture render side-effects for e.g. Ctrl+Q message.
    /// The public-in-module handle_key keeps the run loop and existing calls unchanged.
    #[cfg(test)]
    fn handle_key_with(&mut self, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
        input::handle_key_with(self, out, key)
    }

    /// Smallest helper seam for resize (and testability of it) without redesigning event loop.
    /// Updates screen size, clamps for zero-size safety, reveals cursor, then renders.
    fn handle_resize(&mut self, w: u16, h: u16, out: &mut dyn Write) -> io::Result<()> {
        viewport::handle_resize(self, w, h, out)
    }

    /// Reveal the current cursor row/col so they are visible in the content area.
    /// Called after cursor movement and content mutations (insert, delete, undo/redo).
    /// Clamps first for zero-size terminals so reveal_* see a sane starting point.
    pub(crate) fn reveal_cursor(&mut self) {
        viewport::reveal_cursor(self)
    }

    /// Returns whether (and how) the on-disk file differs from our last captured snapshot.
    /// Used by future watch/reload to decide action; for 2-l this is detection only.
    /// Must not mutate buffer, file state (dirty/snapshot), message, pending, viewport, or history.
    /// NoPath for untitled; delegates to file_state helper (std metadata compare only).
    #[cfg(test)]
    fn external_file_status(&self) -> crate::file::io::ExternalFileStatus {
        external_file_status(&self.file)
    }

    pub(crate) fn render(&self, stdout: &mut dyn Write) -> io::Result<()> {
        // Delegate to terminal render. Render decides the bottom annotation:
        // - if app.message is Some: show the transient (warning/error/quit etc.)
        // - else: show persistent status line (mode/path/dirty/size/tier + large-file marker)
        // App owns the decision string; terminal::render stays generic (receives Option<&str>).
        // Screen is single source for dims.
        // Avoid cloning self.message: pass Some(m.as_str()) directly.
        // Status is built locally only for the no-message path and passed as &str.
        let highlight = (!external_command::is_viewing(self)
            && !view::is_preview(self)
            && !lint::is_viewing(self)
            && !project_files::is_viewing(self)
            && !llm_preview::is_viewing(self))
        .then(|| {
            self.selection
                .active()
                .map(|selection| {
                    let (start, end) = selection.ordered();
                    term::render::TextHighlight { start, end }
                })
                .or_else(|| {
                    self.search
                        .active_match()
                        .map(|found| term::render::TextHighlight {
                            start: found.start,
                            end: crate::buffer::Cursor {
                                row: found.start.row,
                                col: found.end_col,
                            },
                        })
                })
        })
        .flatten();
        let render_options = term::render::RenderOptions {
            highlight,
            syntax: view::display_syntax(self),
            line_numbers: self.view.line_numbers,
            whitespace: self.view.whitespace,
        };
        let display_buffer = view::display_buffer(self);
        if let Some(ref m) = self.message {
            term::render::render_buffer(
                stdout,
                display_buffer,
                term::render::RenderViewport::new(
                    self.screen.scroll_top,
                    self.screen.scroll_left,
                    self.screen.height as usize,
                    self.screen.width as usize,
                ),
                Some(m.as_str()),
                render_options,
            )
        } else {
            let status = status::format_status_line(
                matches!(self.mode, Mode::Plain),
                self.file.path.as_deref(),
                self.file.dirty,
                self.file.size_bytes,
                self.file.size_tier,
                self.buffer.page_info(),
                (self.buffer_count() > 1).then(|| {
                    (
                        self.active_buffer_index.saturating_add(1),
                        self.buffer_count(),
                    )
                }),
            );
            term::render::render_buffer(
                stdout,
                display_buffer,
                term::render::RenderViewport::new(
                    self.screen.scroll_top,
                    self.screen.scroll_left,
                    self.screen.height as usize,
                    self.screen.width as usize,
                ),
                Some(status.as_str()),
                render_options,
            )
        }
    }
}

/// Public entry called from main.rs.
pub fn run(initial_files: &[String]) -> io::Result<()> {
    let big_files = crate::config::big_files::load()?;
    let auto_reload = crate::config::auto_reload::load()?;
    let editor_config = crate::config::editor::load()?;
    let keybindings = crate::config::keybindings::load()?;
    let command_config = crate::config::commands::load()?;
    let mut app = App::new_with_paths_and_config(
        initial_files,
        big_files,
        auto_reload,
        editor_config,
        keybindings,
        command_config,
    )?;
    app.run()
}

#[cfg(test)]
mod tests;
