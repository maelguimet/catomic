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
    /// Updates screen size, reveals cursor row under new height, then renders.
    fn handle_resize(&mut self, w: u16, h: u16, out: &mut dyn Write) -> io::Result<()> {
        self.screen.update_size(w, h);
        self.reveal_cursor();
        self.render(out)
    }

    /// Reveal the current cursor row/col so they are visible in the content area.
    /// Called after cursor movement and content mutations (insert, delete, undo/redo).
    fn reveal_cursor(&mut self) {
        let c = self.buffer.cursor();
        self.screen.reveal_row(c.row);
        self.screen.reveal_col(c.col);
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
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn app_file_state_new_starts_clean() {
        let app = App::new(None).unwrap();
        assert!(!app.file.dirty, "new app without path starts clean");
        assert!(app.file.path.is_none());
        // screen field added in 2-c; verify default here too (no behavior change)
        assert_eq!(app.screen.height, 24);
        assert_eq!(app.screen.scroll_top, 0);

        let app2 = App::new(Some("existing.txt")).unwrap();
        assert!(!app2.file.dirty, "open (even missing file) starts clean");
        assert_eq!(
            app2.file.path.as_deref(),
            Some(std::path::Path::new("existing.txt"))
        );
    }

    #[test]
    fn app_dirty_lifecycle_via_keys() {
        // Use explicit temp path for the test so we NEVER write bare "untitled.txt"
        // into the repo cwd. App::new with a path (even non-existing) starts clean
        // and save will target that path instead of defaulting.
        let mut tmp = std::env::temp_dir();
        tmp.push(format!(
            "catomic_test_dirty_lifecycle_{}_{}.txt",
            std::process::id(),
            "lifecycle"
        ));
        let test_path = tmp.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&test_path); // ensure clean start

        let mut app = App::new(Some(&test_path)).unwrap();
        assert!(!app.file.dirty);
        assert_eq!(
            app.file.path.as_deref(),
            Some(std::path::Path::new(&test_path))
        );

        // char insert marks dirty
        app.handle_key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
        .unwrap();
        assert!(app.file.dirty, "edit marks dirty");

        // save (via atomic) clears dirty; uses explicit path (no untitled.txt)
        app.handle_key(KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
        .unwrap();
        assert!(!app.file.dirty, "successful save marks clean");
        assert!(app.file.path.is_some());

        // edit after save marks dirty again
        app.handle_key(KeyEvent {
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
        .unwrap();
        assert!(app.file.dirty, "post-save edit marks dirty again");

        // Clean up ONLY the temp path created/used by this test.
        let _ = std::fs::remove_file(&test_path);
    }

    // Phase 2-b quit guard + message tests (via simulated keys; no real terminal)

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn app_quit_clean_immediately() {
        let mut app = App::new(None).unwrap();
        assert!(!app.file.dirty);
        assert!(!app.should_quit);
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit, "clean Ctrl+Q quits immediately");
    }

    #[test]
    fn app_quit_dirty_first_sets_pending_and_message_second_quits() {
        let mut app = App::new(None).unwrap();
        // make dirty
        app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.file.dirty);
        assert!(!app.pending_quit_confirm);
        assert!(app.message.is_none());

        // first Ctrl+Q: no quit, sets pending + msg
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(!app.should_quit, "first dirty Q does not quit");
        assert!(app.pending_quit_confirm);
        let msg = app.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("Unsaved changes") && msg.contains("Ctrl+Q again"),
            "message should warn: got {:?}",
            app.message
        );

        // second Ctrl+Q: quits
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit, "second dirty Q quits");
    }

    #[test]
    fn app_dirty_ctrl_q_first_renders_warning_immediately() {
        // Regression for invisible warning: first dirty Ctrl+Q must emit render
        // containing the message on bottom row (via the writer seam).
        let mut app = App::new(None).unwrap();
        app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.file.dirty);
        assert!(app.message.is_none());

        let mut out: Vec<u8> = Vec::new();
        app.handle_key_with(
            &mut out,
            make_key(KeyCode::Char('q'), KeyModifiers::CONTROL),
        )
        .unwrap();

        assert!(!app.should_quit, "first dirty Q does not quit");
        assert!(app.pending_quit_confirm);
        let rendered = String::from_utf8_lossy(&out);
        assert!(
            rendered.contains("Unsaved changes") && rendered.contains("Ctrl+Q again"),
            "warning message text must appear in render output"
        );
        assert!(
            rendered.contains("\x1b[K"),
            "render must clear bottom row with \\x1b[K even for message"
        );
    }

    #[test]
    fn app_ctrl_s_after_dirty_clears_dirty_and_pending() {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!(
            "catomic_test_save_clears_pending_{}.txt",
            std::process::id()
        ));
        let p = tmp.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);

        let mut app = App::new(Some(&p)).unwrap();
        app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.file.dirty);

        // trigger quit warn
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.pending_quit_confirm);

        // Ctrl+S: success clears dirty + pending + msg
        app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(!app.file.dirty);
        assert!(!app.pending_quit_confirm);
        assert!(app.message.is_none());

        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn app_save_error_keeps_dirty_and_sets_error_message() {
        // Use a dedicated subdir under temp (never bare temp_dir or root sibling)
        // so that path points to a directory -> atomic_write fails as intended.
        let mut bad = std::env::temp_dir();
        bad.push(format!("catomic_bad_save_dir_{}", std::process::id()));
        // ensure clean and is a dir
        let _ = std::fs::remove_dir_all(&bad);
        std::fs::create_dir_all(&bad).expect("create dedicated bad dir");
        assert!(bad.is_dir());

        let mut app = App::new(None).unwrap();
        app.file.path = Some(bad.clone());
        app.file.dirty = true;
        app.message = None;

        app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();

        assert!(app.file.dirty, "save error must keep dirty=true");
        let msg = app.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("Save error") || msg.contains("error"),
            "save error should set message, got: {:?}",
            app.message
        );

        // cleanup dedicated dir only
        let _ = std::fs::remove_dir_all(&bad);
    }

    #[test]
    fn app_edit_after_quit_warning_clears_pending() {
        let mut app = App::new(None).unwrap();
        app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.file.dirty);

        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.pending_quit_confirm);
        assert!(app.message.is_some());

        // content-mutating edit clears BOTH pending and message (movements do not)
        app.handle_key(make_key(KeyCode::Char('!'), KeyModifiers::NONE))
            .unwrap();
        assert!(
            !app.pending_quit_confirm,
            "edit after warning clears pending"
        );
        assert!(
            app.message.is_none(),
            "edit after warning also clears stale message"
        );
    }

    #[test]
    fn app_new_has_default_screen_size_and_scroll() {
        let app = App::new(None).unwrap();
        assert_eq!(app.screen.width, 80, "default width");
        assert_eq!(
            app.screen.height, 24,
            "default height (matches prior hardcoded)"
        );
        assert_eq!(app.screen.scroll_top, 0);
    }

    #[test]
    fn app_render_respects_screen_height_via_captured_writer() {
        let mut app = App::new(None).unwrap();
        // set non-default height (no real term)
        app.screen.height = 10;
        app.screen.scroll_top = 0;

        // trigger render via content path that calls render (uses handle_key_with seam)
        let mut out: Vec<u8> = Vec::new();
        app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();

        let rendered = String::from_utf8_lossy(&out);
        // bottom row clear/pos for height=10
        assert!(
            rendered.contains("\x1b[10;1H"),
            "render must use screen height for bottom row positioning"
        );
        assert!(rendered.contains("\x1b[K"), "clears using \\x1b[K");
    }

    #[test]
    fn app_handle_resize_updates_screen_and_renders() {
        let mut app = App::new(None).unwrap();
        assert_eq!(app.screen.height, 24);

        let mut out: Vec<u8> = Vec::new();
        app.handle_resize(50, 15, &mut out).unwrap();

        assert_eq!(app.screen.width, 50);
        assert_eq!(app.screen.height, 15);
        let rendered = String::from_utf8_lossy(&out);
        assert!(
            rendered.contains("\x1b[15;1H"),
            "resize render must position using new screen height"
        );
        assert!(!out.is_empty(), "resize must have triggered a render");
    }

    // Phase 2-d app-level reveal/scroll_top tests (via seams + captured render)

    #[test]
    fn app_cursor_down_past_visible_updates_scroll_top() {
        let mut app = App::new(None).unwrap();
        // Small content viewport: height=6 => visible_height=5 content rows (0..4)
        app.screen.height = 6;
        app.screen.scroll_top = 0;

        // Create 10 lines (0..9) by newlines; cursor ends after last insert at end of last line.
        // Use Enter key via seam to exercise the path that does reveal (captures output, keeps test quiet).
        let mut sink: Vec<u8> = Vec::new();
        for _ in 0..9 {
            app.handle_key_with(&mut sink, make_key(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap();
        }
        // Now we have 10 lines (rows 0-9), cursor at row=9, col=0 (after 9 newlines from empty start)
        assert_eq!(app.buffer.cursor().row, 9);

        // With vh=5, row 9 is way below (0+5=5), so reveal must have scrolled on last Enter.
        // scroll_top should be at least 9 +1 -5 = 5
        assert!(
            app.screen.scroll_top >= 5,
            "down past viewport must update scroll_top; got {}",
            app.screen.scroll_top
        );
    }

    #[test]
    fn app_render_after_reveal_omits_earlier_lines_and_shows_cursor_row() {
        let mut app = App::new(None).unwrap();
        app.screen.height = 6; // vh=5
        app.screen.scroll_top = 0;

        // Build lines with unique markers: insert "L0\nL1\n...L9"
        // Simpler: repeated Enter then type a marker char on each line? Use direct buffer for setup clarity.
        // Then drive a Down that will reveal via the key path.
        for i in 0..10 {
            if i > 0 {
                app.buffer.insert_newline();
            }
            // put a distinguishable token at start of each line
            app.buffer.insert_char('L');
            // i as rough marker by repeating a char; keep simple: use digits for later lines
            let marker = char::from(b'0' + (i % 10) as u8);
            app.buffer.insert_char(marker);
        }
        // cursor now at row=9, col=2 on "L9"
        assert_eq!(app.buffer.cursor().row, 9);

        // Force a scroll by simulating many downs via keys (each calls reveal_cursor)
        // Use handle_key_with + sink to exercise reveal path without spamming test stdout.
        let mut sink: Vec<u8> = Vec::new();
        // Start from top by resetting scroll; then down past.
        app.screen.scroll_top = 0;
        // Move up to row 0 first (we are at 9), then down 9 times with small vh to trigger reveal on the way.
        for _ in 0..9 {
            app.handle_key_with(&mut sink, make_key(KeyCode::Up, KeyModifiers::NONE))
                .unwrap();
        }
        assert_eq!(app.buffer.cursor().row, 0);
        app.screen.scroll_top = 0;

        // Now move down past the visible area
        for _ in 0..9 {
            app.handle_key_with(&mut sink, make_key(KeyCode::Down, KeyModifiers::NONE))
                .unwrap();
        }
        assert_eq!(app.buffer.cursor().row, 9);
        assert!(
            app.screen.scroll_top > 0,
            "must have scrolled; scroll_top={}",
            app.screen.scroll_top
        );

        // Capture a render; earlier lines (e.g. L0) must not be in the emitted content region.
        let mut out: Vec<u8> = Vec::new();
        app.render(&mut out).unwrap();
        let rendered = String::from_utf8_lossy(&out);

        // The render writes visible_lines(scroll_top, content_h). First line content after clear should not be L0/L1 if scrolled.
        // Check absence of a unique early marker that would be before scroll_top.
        assert!(
            !rendered.contains("L0"),
            "early line content must not be emitted when scrolled; scroll_top={}\nout: {}",
            app.screen.scroll_top,
            rendered
        );
        // Cursor row's content should be present (L9 or similar)
        assert!(
            rendered.contains("L9"),
            "cursor row content must be emitted; got scroll_top={} rendered=\n{}",
            app.screen.scroll_top,
            rendered
        );
    }

    #[test]
    fn app_resize_smaller_reveals_cursor_row() {
        let mut app = App::new(None).unwrap();
        // Create 16 lines (0..15) with cursor at row 15
        for _ in 0..15 {
            app.buffer.insert_newline();
        }
        assert_eq!(app.buffer.cursor().row, 15);
        // Large viewport so currently no scroll
        app.screen.height = 30;
        app.screen.scroll_top = 0;

        // Now resize to a small height where 15 would be offscreen if not revealed.
        // height=10 => vh=9; 15 >= 0+9 => reveal will set scroll_top = 15+1-9=7
        let mut out: Vec<u8> = Vec::new();
        app.handle_resize(40, 10, &mut out).unwrap();

        assert_eq!(app.screen.height, 10);
        assert!(
            app.screen.scroll_top > 0,
            "resize to smaller must reveal; scroll_top={}",
            app.screen.scroll_top
        );
        // 15 should now be inside [scroll_top, scroll_top+8]
        let vh = app.screen.visible_height();
        assert!(
            app.screen.scroll_top <= 15 && 15 < app.screen.scroll_top + vh,
            "cursor row 15 must be visible after small resize; scroll_top={}, vh={}",
            app.screen.scroll_top,
            vh
        );
        assert!(!out.is_empty(), "resize must render");
    }
}
