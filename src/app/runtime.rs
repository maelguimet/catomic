//! Purpose: run the single terminal event loop and dispatch normalized terminal events.
//! Owns: setup/teardown guards, runtime polling order, event reads, and resize dispatch.
//! Must not: decode terminal bytes, implement editor commands, scan projects, or call networks.
//! Invariants: background work is polled once per loop; terminal teardown is guard-backed.
//! Phase: bounded post-beta App ownership cleanup.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyEvent};

use crate::terminal as term;

use super::{
    autocomplete, command_prompt, external_command, hooks, inline_clanker, input, lint,
    llm_request, model_picker, project_files, recovery, repo_llm, search, selection, viewport,
    watch, App,
};

impl App {
    /// The main goblin loop. Keep it obvious.
    pub fn run(&mut self) -> io::Result<()> {
        self.run_with_startup_config(None)
    }

    pub(super) fn run_config(&mut self, path: PathBuf) -> io::Result<()> {
        self.run_with_startup_config(Some(path))
    }

    fn run_with_startup_config(&mut self, config_path: Option<PathBuf>) -> io::Result<()> {
        let mut stdout = io::stdout();
        let terminal_guard = term::TerminalGuard::new();
        terminal_guard.setup(&mut stdout)?;
        if let Ok((width, height)) = crossterm::terminal::size() {
            self.screen.update_size(width, height);
        }
        let _panic_guard = term::PanicRestoreGuard::install(terminal_guard.restorer());
        if let Some(path) = config_path {
            command_prompt::open_startup_config(self, &mut stdout, path)?;
        } else {
            hooks::trigger_open(self);
            if autocomplete::configured_default_enabled(self) {
                autocomplete::begin_enable(self, &mut stdout)?;
            } else {
                self.render(&mut stdout)?;
            }
        }

        while !self.should_quit && term::termination_signal().is_none() {
            if term::take_resize_pending() {
                let (width, height) = crossterm::terminal::size()?;
                if (width, height) != (self.screen.width, self.screen.height) {
                    self.handle_resize(width, height, &mut stdout)?;
                }
            }
            self.poll_runtime_tasks(&mut stdout)?;
            if event::poll(std::time::Duration::from_millis(100))? {
                self.dispatch_terminal_event(&mut stdout, event::read()?)?;
            }
        }

        terminal_guard.restore(&mut stdout)?;
        Ok(())
    }

    fn poll_runtime_tasks(&mut self, out: &mut dyn Write) -> io::Result<()> {
        watch::check_file_watcher_once_and_render(self, out)?;
        search::poll_search(self, out)?;
        command_prompt::poll_goto(self, out)?;
        lint::poll(self, out)?;
        project_files::poll(self, out)?;
        model_picker::poll(self, out)?;
        llm_request::poll(self, out)?;
        inline_clanker::poll(self, out)?;
        repo_llm::poll(self, out)?;
        external_command::poll(self, out)?;
        hooks::pump(self, out)?;
        recovery::poll(self, out)?;
        autocomplete::poll(self, out)
    }

    fn dispatch_terminal_event(&mut self, out: &mut dyn Write, event: Event) -> io::Result<()> {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Paste(text) => input::handle_paste(self, out, &text),
            Event::Mouse(mouse) => selection::handle_mouse(self, out, mouse),
            Event::Resize(width, height) => self.handle_resize(width, height, out),
            Event::FocusGained => {
                viewport::redraw_after_focus(self, crossterm::terminal::size().ok(), out)
            }
            Event::FocusLost => Ok(()),
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        input::handle_key(self, key)
    }

    #[cfg(test)]
    pub(super) fn handle_key_with(&mut self, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
        input::handle_key_with(self, out, key)
    }

    pub(super) fn handle_resize(
        &mut self,
        width: u16,
        height: u16,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        viewport::handle_resize(self, width, height, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};

    fn control(character: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(character),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn copy_times_out_stuck_helper_falls_back_and_keeps_input_responsive() {
        crate::clipboard::with_timeout_test_helpers(|| {
            let text = "x".repeat(100 * 1024 + 1);
            let mut app = App::new(None).unwrap();
            app.buffer = Box::new(crate::buffer::PieceTable::from_text(&text));
            let mut out = Vec::new();
            app.handle_key_with(&mut out, control('a')).unwrap();

            let started = Instant::now();
            app.handle_key_with(&mut out, control('c')).unwrap();

            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(app.clipboard, text);
            assert!(app.message.is_none(), "the fallback helper was not tried");

            app.handle_key_with(&mut out, control('q')).unwrap();
            assert!(app.should_quit);
        });
    }
}
