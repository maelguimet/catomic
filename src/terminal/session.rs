//! Purpose: own the paired lifetime of terminal modes used by an editor session.
//! Owns: alternate-screen, enhanced-keyboard, bracketed-paste, mouse, and raw-mode setup.
//! Must not: decode input, interpret editor commands, render content, or mutate App state.
//! Invariants: keyboard flags are pushed inside the alternate screen and popped there once.
//! Phase: post-v0.1 terminal keyboard compatibility.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crossterm::event::KeyboardEnhancementFlags;

const ALTERNATE_SCREEN: u8 = 1 << 0;
const KEYBOARD_FLAGS: u8 = 1 << 1;
const RESTORING: u8 = 1 << 7;

pub(crate) const KEYBOARD_FLAGS_REQUEST: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES);

/// Restores a single editor session. Clones coordinate panic and Drop cleanup.
#[derive(Clone)]
pub(crate) struct TerminalRestorer {
    active_modes: Arc<AtomicU8>,
}

/// Guard installed before the first terminal mutation.
pub(crate) struct TerminalGuard {
    restorer: TerminalRestorer,
}

impl TerminalGuard {
    pub(crate) fn new() -> Self {
        Self {
            restorer: TerminalRestorer {
                active_modes: Arc::new(AtomicU8::new(0)),
            },
        }
    }

    pub(crate) fn setup<W: Write>(&self, out: &mut W) -> io::Result<()> {
        self.enable_output_modes(out)?;
        crossterm::terminal::enable_raw_mode()
    }

    pub(crate) fn restore<W: Write>(&self, out: &mut W) -> io::Result<()> {
        self.restorer.restore(out)
    }

    pub(crate) fn restorer(&self) -> TerminalRestorer {
        self.restorer.clone()
    }

    fn enable_output_modes<W: Write>(&self, out: &mut W) -> io::Result<()> {
        use crossterm::{cursor, event, execute, terminal};

        execute!(out, terminal::EnterAlternateScreen)?;
        self.restorer.mark_active(ALTERNATE_SCREEN);
        execute!(
            out,
            event::PushKeyboardEnhancementFlags(KEYBOARD_FLAGS_REQUEST)
        )?;
        self.restorer.mark_active(KEYBOARD_FLAGS);
        execute!(
            out,
            event::EnableBracketedPaste,
            event::EnableMouseCapture,
            cursor::Show
        )
    }

    #[cfg(test)]
    pub(crate) fn enable_output_modes_for_test<W: Write>(&self, out: &mut W) -> io::Result<()> {
        self.enable_output_modes(out)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restorer.restore(&mut io::stdout());
    }
}

impl TerminalRestorer {
    pub(crate) fn restore_stdout(&self) {
        let _ = self.restore(&mut io::stdout());
    }

    fn mark_active(&self, mode: u8) {
        self.active_modes.fetch_or(mode, Ordering::Release);
    }

    pub(crate) fn restore<W: Write>(&self, out: &mut W) -> io::Result<()> {
        let Some(active) = self.begin_restore() else {
            return Ok(());
        };
        let _ = crossterm::terminal::disable_raw_mode();
        let (remaining, result) = restore_output_modes(out, active);
        self.active_modes.store(remaining, Ordering::Release);
        result
    }

    fn begin_restore(&self) -> Option<u8> {
        loop {
            let active = self.active_modes.load(Ordering::Acquire);
            if active == 0 || active == RESTORING {
                return None;
            }
            if self
                .active_modes
                .compare_exchange(active, RESTORING, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(active);
            }
        }
    }
}

fn restore_output_modes<W: Write>(out: &mut W, active: u8) -> (u8, io::Result<()>) {
    use crossterm::{cursor, event, execute, terminal};

    let mut remaining = active;
    let mut first_error = crate::terminal::cursor_style::restore(out).err();
    if let Err(error) = write!(out, "\x1b[0m\x1b]112\x07") {
        first_error.get_or_insert(error);
    }
    if let Err(error) = execute!(
        out,
        event::DisableMouseCapture,
        event::DisableBracketedPaste,
        cursor::Show
    ) {
        first_error.get_or_insert(error);
    }
    if active & KEYBOARD_FLAGS != 0 {
        match execute!(out, event::PopKeyboardEnhancementFlags) {
            Ok(()) => remaining &= !KEYBOARD_FLAGS,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if remaining & KEYBOARD_FLAGS == 0 && active & ALTERNATE_SCREEN != 0 {
        match execute!(out, terminal::LeaveAlternateScreen) {
            Ok(()) => remaining &= !ALTERNATE_SCREEN,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    (remaining, first_error.map_or(Ok(()), Err))
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
