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

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{self, Buffer};
use crate::mode::{Capabilities, Mode};
use crate::terminal as term;

/// High-level application state for the editor.
pub struct App {
    pub mode: Mode,
    pub caps: Capabilities,
    /// The active buffer (trait object for now; concrete type behind it).
    pub buffer: Box<dyn Buffer>,
    /// Path of the file being edited, if any.
    pub file_path: Option<String>,
    /// Whether we should exit the loop.
    pub should_quit: bool,
}

impl App {
    pub fn new(initial_path: Option<&str>) -> io::Result<Self> {
        let mode = Mode::Plain; // Start in Plain by default. User can switch later.
        let caps = Capabilities::from_mode(mode);

        let buffer: Box<dyn Buffer> = if let Some(path) = initial_path {
            // Phase 0: dead-simple load into SimpleBuffer
            let content = std::fs::read_to_string(path).unwrap_or_default();
            Box::new(buffer::SimpleBuffer::from_text(&content))
        } else {
            Box::new(buffer::SimpleBuffer::new())
        };

        Ok(App {
            mode,
            caps,
            buffer,
            file_path: initial_path.map(|s| s.to_string()),
            should_quit: false,
        })
    }

    /// The main goblin loop. Keep it obvious.
    pub fn run(&mut self) -> io::Result<()> {
        // Terminal setup is in the terminal module.
        let mut stdout = io::stdout();
        term::setup(&mut stdout)?;

        // Ensure terminal is restored on panic or early return.
        // The guard calls teardown on drop (unwind or normal).
        let _guard = term::TerminalGuard::new();

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

        // Explicit teardown before guard also drops (idempotent).
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

            // Save (Ctrl+S)
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                let path = self
                    .file_path
                    .clone()
                    .unwrap_or_else(|| "untitled.txt".to_string());
                let text = self.buffer.to_string();
                // Phase 0: simple write. If no prior path we now remember "untitled.txt".
                std::fs::write(&path, text)?;
                // Remember the path so subsequent saves work without picking default again.
                if self.file_path.is_none() {
                    self.file_path = Some(path);
                }
                self.render(&mut io::stdout())?;
            }

            // Basic movement + editing (Phase 0)
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if c == '\n' || c == '\r' {
                    self.buffer.insert_newline();
                } else {
                    self.buffer.insert_char(c);
                }
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.buffer.delete_back();
                self.render(&mut io::stdout())?;
            }

            KeyEvent {
                code: KeyCode::Delete,
                ..
            } => {
                self.buffer.delete_forward();
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

            // TODO: Enter, arrows, page up/down, etc.
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
