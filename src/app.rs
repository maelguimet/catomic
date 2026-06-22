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
                    Event::Resize(_w, _h) => {
                        // Phase 0 ignores resize for simplicity (see TODO).
                        // Later: re-render with new dimensions.
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
        match key {
            // Quit
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.should_quit = true;
            }

            // Save (Ctrl+S) -- now routes through atomic write. Save never creates undo.
            // If no path, defaults to "untitled.txt" and remembers it.
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
                file::io::atomic_write_string(&path, &text)?;
                if self.file.path.is_none() {
                    self.file.path = Some(path);
                }
                self.file.dirty = false;
                self.render(&mut io::stdout())?;
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
                self.render(&mut io::stdout())?;
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
                self.render(&mut io::stdout())?;
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
                self.render(&mut io::stdout())?;
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
                self.render(&mut io::stdout())?;
            }
            KeyEvent {
                code: KeyCode::Char('y'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.redo();
                self.file.dirty = true;
                self.render(&mut io::stdout())?;
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
                } else if !c.is_control() {
                    let ch = if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    self.buffer.insert_char(ch);
                    self.file.dirty = true;
                }
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.buffer.delete_back();
                self.file.dirty = true;
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Delete,
                ..
            } => {
                self.buffer.delete_forward();
                self.file.dirty = true;
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Left,
                ..
            } => {
                self.buffer.move_left();
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Right,
                ..
            } => {
                self.buffer.move_right();
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.buffer.move_up();
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.buffer.move_down();
                self.render(&mut io::stdout())?;
            }

            _ => {}
        }

        Ok(())
    }

    fn render(&self, stdout: &mut dyn Write) -> io::Result<()> {
        // Delegate to terminal render for Phase 0. Keep the loop caller simple.
        term::render::render_buffer(stdout, &*self.buffer, 0, 24)
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

        let app2 = App::new(Some("existing.txt")).unwrap();
        assert!(!app2.file.dirty, "open (even missing file) starts clean");
        assert_eq!(app2.file.path.as_deref(), Some(std::path::Path::new("existing.txt")));
    }

    #[test]
    fn app_dirty_lifecycle_via_keys() {
        let mut app = App::new(None).unwrap();
        assert!(!app.file.dirty);

        // char insert marks dirty
        app.handle_key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
        .unwrap();
        assert!(app.file.dirty, "edit marks dirty");

        // save (via atomic) clears dirty, sets default path
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
    }
}
