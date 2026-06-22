//! App state + the one blessed goblin loop.
//!
//! Per TODO.md:
//! - "Keep the main (goblin) loop extremely boring and in one obvious place."
//! - Phase 0: ultra-minimal MVP. Cursor, insert, delete, open, save, quit.
//! - Buffer trait lives in `buffer`.
//!
//! This module owns high-level state (current buffer, mode, capabilities,
//! terminal handle, etc.) and the event loop.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{self, Buffer};
use crate::file;
use crate::mode::{Capabilities, Mode};
use crate::terminal as term;

/// Minimal explicit file state (Phase 2-a).
/// path: target for save (None until first save picks "untitled.txt").
/// dirty: true if buffer has unsaved edits since last save/open.
/// Starts false for open-existing and open-missing-file cases.
#[derive(Clone, Debug, Default)]
pub struct FileState {
    pub path: Option<PathBuf>,
    pub dirty: bool,
}

/// High-level application state for the editor.
pub struct App {
    pub mode: Mode,
    pub caps: Capabilities,
    /// The active buffer (trait object for now; concrete type behind it).
    pub buffer: Box<dyn Buffer>,
    /// File path and dirty tracking.
    pub file: FileState,
    /// Whether we should exit the loop.
    pub should_quit: bool,
    /// Minimal message for user (error, quit warning, etc.). Cleared on edits or explicit.
    pub message: Option<String>,
    /// When true, a second Ctrl+Q while dirty will force quit (no save).
    pub pending_quit_confirm: bool,
    /// Terminal screen size and scroll state. Single source of truth for render height.
    /// Initialized conservatively; updated from crossterm after setup and on resize.
    pub screen: term::screen::Screen,
}

impl App {
    pub fn new(initial_path: Option<&str>) -> io::Result<Self> {
        let mode = Mode::Plain; // Start in Plain by default. User can switch later.
        let caps = Capabilities::from_mode(mode);

        let buffer: Box<dyn Buffer> = if let Some(path) = initial_path {
            // Distinguish missing file (start empty, but remember path so save creates it)
            // from real errors (permission, utf8, is-dir, etc). Silent empty was data-loss bait.
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(e),
            };
            Box::new(buffer::PieceTable::from_text(&content))
        } else {
            Box::new(buffer::PieceTable::new())
        };

        Ok(App {
            mode,
            caps,
            buffer,
            file: FileState {
                path: initial_path.map(|s| PathBuf::from(s)),
                dirty: false,
            },
            should_quit: false,
            message: None,
            pending_quit_confirm: false,
            // Conservative default matching prior hardcoded 24; no real term required for unit tests.
            screen: term::screen::Screen::new(80, 24),
        })
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
        // We chain to the previous hook.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Try immediate restore.
            let _ = term::teardown(&mut io::stdout());
            prev_hook(info);
        }));

        // Phase 0 render is extremely dumb.
        self.render(&mut stdout)?;

        while !self.should_quit {
            // Blocking read for Phase 0. Later we may need non-blocking + resize.
            if event::poll(std::time::Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) => {
                        self.handle_key(key)?;
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
        let mut out = io::stdout();
        self.handle_key_with(&mut out, key)
    }

    /// Route key handling + associated renders through a writer.
    /// Smallest seam so tests can capture render side-effects for e.g. Ctrl+Q message.
    /// The public-in-module handle_key keeps the run loop and existing calls unchanged.
    fn handle_key_with(&mut self, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
        match key {
            // Quit (Ctrl+Q)
            // - clean: quit immediately
            // - dirty + !pending: set pending=true + warning message; do NOT quit
            // - dirty + pending: quit (force, without save)
            // Movement keys leave pending/message as-is (simplest behavior; documented).
            // Actual content-mutating edits (insert/delete/undo/redo) clear BOTH pending_confirm and message
            // (so stale quit warnings disappear after typing). Save success also clears them.
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                if !self.file.dirty {
                    self.should_quit = true;
                } else if self.pending_quit_confirm {
                    self.should_quit = true;
                } else {
                    self.pending_quit_confirm = true;
                    self.message = Some(
                        "Unsaved changes. Press Ctrl+Q again to quit without saving, Ctrl+S to save."
                            .to_string(),
                    );
                    self.render(out)?;
                    // do not quit
                }
            }

            // Save (Ctrl+S) -- now routes through atomic write. Save never creates undo.
            // If no path, defaults to "untitled.txt" and remembers it.
            // On success: dirty=false, clear pending + message.
            // On error: keep dirty=true, set short error message, do not panic.
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                let path = self
                    .file
                    .path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("untitled.txt"));
                let text = self.buffer.to_string();
                match file::io::atomic_write_string(&path, &text) {
                    Ok(()) => {
                        if self.file.path.is_none() {
                            self.file.path = Some(path);
                        }
                        self.file.dirty = false;
                        self.pending_quit_confirm = false;
                        self.message = None;
                    }
                    Err(e) => {
                        self.message = Some(format!("Save error: {}", e));
                        // keep dirty; do not clear pending (if user had quit warn, error is shown)
                    }
                }
                self.render(out)?;
            }

            // Enter produces KeyCode::Enter (not Char('\n')). Handle explicitly.
            // The Char \n/\r check below catches any that might arrive via paste
            // or other terminal paths.
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                self.buffer.insert_newline();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }

            // Undo / Redo (Phase 1C). Ctrl+Z undo; Ctrl+Y and Ctrl+Shift+Z redo.
            // Redo must handle both common terminal reports for Ctrl+Shift+Z:
            //   - KeyCode::Char('z') + CONTROL + SHIFT
            //   - KeyCode::Char('Z') + CONTROL + SHIFT
            // Place before generic Char so CONTROL combos fire. No other UI changes.
            // Mark dirty conservatively (undo/redo can mutate); exact save-point
            // tracking is future.
            KeyEvent {
                code: KeyCode::Char('z'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.buffer.undo();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }
            KeyEvent {
                code: KeyCode::Char('z'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                && modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.buffer.redo();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }
            KeyEvent {
                code: KeyCode::Char('Z'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                && modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.buffer.redo();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }
            KeyEvent {
                code: KeyCode::Char('y'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.redo();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }

            // Basic movement + editing (Phase 0)
            // Accept any Char that is not control. Apply SHIFT modifier for
            // uppercase letters (crossterm may report lowercase + SHIFT).
            // Specific Ctrl+S / Ctrl+Q arms above take precedence for CONTROL.
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    // Other Ctrl+letter combos ignored in Phase 0
                } else if c == '\n' || c == '\r' {
                    self.buffer.insert_newline();
                    self.file.dirty = true;
                    self.pending_quit_confirm = false;
                    self.message = None;
                } else if !c.is_control() {
                    let ch = if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    self.buffer.insert_char(ch);
                    self.file.dirty = true;
                    self.pending_quit_confirm = false;
                    self.message = None;
                }
                self.reveal_cursor();
                self.render(out)?;
            }

            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.buffer.delete_back();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }

            KeyEvent {
                code: KeyCode::Delete,
                ..
            } => {
                self.buffer.delete_forward();
                self.file.dirty = true;
                self.pending_quit_confirm = false;
                self.message = None;
                self.reveal_cursor();
                self.render(out)?;
            }

            KeyEvent {
                code: KeyCode::Left,
                ..
            } => {
                self.buffer.move_left();
                self.reveal_cursor();
                self.render(out)?;
            }

            KeyEvent {
                code: KeyCode::Right,
                ..
            } => {
                self.buffer.move_right();
                self.reveal_cursor();
                self.render(out)?;
            }

            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.buffer.move_up();
                self.reveal_cursor();
                self.render(out)?;
            }

            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.buffer.move_down();
                self.reveal_cursor();
                self.render(out)?;
            }

            _ => {}
        }

        Ok(())
    }

    /// Smallest helper seam for resize (and testability of it) without redesigning event loop.
    /// Updates screen size, clamps for zero-size safety, reveals cursor, then renders.
    fn handle_resize(&mut self, w: u16, h: u16, out: &mut dyn Write) -> io::Result<()> {
        self.screen.update_size(w, h);
        self.screen.clamp_scroll();
        self.clamp_viewport_to_buffer();
        self.reveal_cursor();
        self.render(out)
    }

    /// Reveal the current cursor row/col so they are visible in the content area.
    /// Called after cursor movement and content mutations (insert, delete, undo/redo).
    /// Clamps first for zero-size terminals so reveal_* see a sane starting point.
    fn reveal_cursor(&mut self) {
        self.screen.clamp_scroll();
        self.clamp_viewport_to_buffer();
        let c = self.buffer.cursor();
        self.screen.reveal_row(c.row);
        self.screen.reveal_col(c.col);
        // Re-clamp after reveal: reveal_col may target a col on a now-shorter line,
        // leaving scroll_left > (line_len - vw). Clamp pulls it back.
        self.clamp_viewport_to_buffer();
    }

    /// Buffer-aware clamp so scroll offsets cannot exceed useful buffer content.
    /// Vertical: if vh==0 => 0; if line_count <= vh => 0; else scroll_top <= line_count - vh.
    /// Horizontal (scalar chars): clamp scroll_left using current cursor line char count.
    /// Uses line_len + 1 - vw (saturating) to match reveal_col end-of-line math and
    /// keep cursor revealed; clamps excess from prior long lines after move/delete/shrink.
    /// Private App helper keeps Screen buffer-agnostic.
    /// Called from resize and reveal paths.
    fn clamp_viewport_to_buffer(&mut self) {
        // Vertical
        let lc = self.buffer.line_count();
        let vh = self.screen.visible_height();
        if vh == 0 {
            self.screen.scroll_top = 0;
        } else if lc <= vh {
            self.screen.scroll_top = 0;
        } else {
            let max_top = lc - vh;
            if self.screen.scroll_top > max_top {
                self.screen.scroll_top = max_top;
            }
        }

        // Horizontal: scalar char count on the *current cursor line* only.
        // Matches Phase 2 scalar limitation; movement/delete/undo to shorter line
        // must not leave scroll_left stranded.
        // Max uses line_len + 1 - vw (saturating) to be consistent with reveal_col's
        // "col + 1 - vw" for end-of-line cursor (col can == line_len). This preserves
        // reveal behavior and keeps cursor visible while still clamping high scrolls
        // on shorter lines to avoid empty space.
        let c = self.buffer.cursor();
        let line_len = self
            .buffer
            .line(c.row)
            .map(|s| s.chars().count())
            .unwrap_or(0);
        let vw = self.screen.visible_width();
        if vw == 0 {
            self.screen.scroll_left = 0;
        } else {
            let max_left = line_len.saturating_add(1).saturating_sub(vw);
            if self.screen.scroll_left > max_left {
                self.screen.scroll_left = max_left;
            }
        }
    }

    fn render(&self, stdout: &mut dyn Write) -> io::Result<()> {
        // Delegate to terminal render. Pass message for bottom-line display.
        // Use screen as single source for height/scroll (no more hardcoded 24).
        // Minimal: only message text (no filename/dirty marker etc).
        term::render::render_buffer(
            stdout,
            &*self.buffer,
            self.screen.scroll_top,
            self.screen.scroll_left,
            self.screen.height as usize,
            self.screen.width as usize,
            self.message.as_deref(),
        )
    }
}

/// Public entry called from main.rs.
pub fn run(initial_file: Option<&str>) -> io::Result<()> {
    let mut app = App::new(initial_file)?;
    app.run()
}

#[cfg(test)]
mod tests;
