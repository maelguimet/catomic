//! Purpose: own the paired lifetime of terminal modes used by an editor session.
//! Owns: alternate-screen, enhanced-keyboard, bracketed-paste, mouse, and raw-mode setup.
//! Must not: decode input, interpret editor commands, render content, or mutate App state.
//! Invariants: each negotiated keyboard mode is reset once before alternate-screen exit;
//!   teardown first releases any interrupted synchronized render update.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crossterm::event::KeyboardEnhancementFlags;

const ALTERNATE_SCREEN: u8 = 1 << 0;
const KITTY_KEYBOARD_FLAGS: u8 = 1 << 1;
const XTERM_EXTENDED_KEYS: u8 = 1 << 2;
const TITLE_STACK: u8 = 1 << 3;
const XTERM_OTHER_KEYS_FORMAT: u8 = 1 << 4;
const RESTORING: u8 = 1 << 7;

const XTERM_EXTENDED_KEYS_ENABLE: &[u8] = b"\x1b[>4;2m";
// Omitting the value restores the terminal's configured initial value. An
// explicit `;0` would instead clobber a non-default user setting on exit.
const XTERM_EXTENDED_KEYS_RESET: &[u8] = b"\x1b[>4m";
const XTERM_OTHER_KEYS_FORMAT_CSI_U: &[u8] = b"\x1b[>4;1f";
const XTERM_OTHER_KEYS_FORMAT_RESET: &[u8] = b"\x1b[>4f";
const TITLE_STACK_PUSH: &[u8] = b"\x1b[22;0t";
const TITLE_STACK_POP: &[u8] = b"\x1b[23;0t";

pub(crate) const KEYBOARD_FLAGS_REQUEST: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
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
        out.write_all(TITLE_STACK_PUSH)?;
        self.restorer.mark_active(TITLE_STACK);
        execute!(
            out,
            event::PushKeyboardEnhancementFlags(KEYBOARD_FLAGS_REQUEST)
        )?;
        self.restorer.mark_active(KITTY_KEYBOARD_FLAGS);
        // Kitty's disambiguation flag intentionally leaves Backspace on its
        // legacy encoding. Request xterm modifyOtherKeys level 2 as the
        // complementary path in every session, not only under tmux. Terminals
        // that do not implement it ignore the well-formed CSI sequence.
        // Crossterm decodes the CSI-u form, so request that format before
        // enabling modifyOtherKeys; xterm otherwise defaults to CSI 27;...~.
        out.write_all(XTERM_OTHER_KEYS_FORMAT_CSI_U)?;
        self.restorer.mark_active(XTERM_OTHER_KEYS_FORMAT);
        out.write_all(XTERM_EXTENDED_KEYS_ENABLE)?;
        self.restorer.mark_active(XTERM_EXTENDED_KEYS);
        out.flush()?;
        execute!(
            out,
            event::EnableBracketedPaste,
            event::EnableFocusChange,
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

    if let Err(error) = out.write_all(crate::terminal::render::TERMINAL_STATE_RECOVERY) {
        return (active, Err(error));
    }
    if let Err(error) = out.write_all(crate::terminal::render::SYNC_UPDATE_END) {
        return (active, Err(error));
    }
    let mut remaining = active;
    let mut first_error = None;
    if let Err(error) = crate::terminal::cursor_style::restore(out) {
        first_error.get_or_insert(error);
    }
    if let Err(error) = write!(out, "\x1b[0m\x1b]112\x07") {
        first_error.get_or_insert(error);
    }
    if let Err(error) = execute!(
        out,
        event::DisableMouseCapture,
        event::DisableFocusChange,
        event::DisableBracketedPaste,
        cursor::Show
    ) {
        first_error.get_or_insert(error);
    }
    if active & XTERM_EXTENDED_KEYS != 0 {
        match out
            .write_all(XTERM_EXTENDED_KEYS_RESET)
            .and_then(|()| out.flush())
        {
            Ok(()) => remaining &= !XTERM_EXTENDED_KEYS,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if active & XTERM_OTHER_KEYS_FORMAT != 0 {
        match out
            .write_all(XTERM_OTHER_KEYS_FORMAT_RESET)
            .and_then(|()| out.flush())
        {
            Ok(()) => remaining &= !XTERM_OTHER_KEYS_FORMAT,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if active & KITTY_KEYBOARD_FLAGS != 0 {
        match execute!(out, event::PopKeyboardEnhancementFlags) {
            Ok(()) => remaining &= !KITTY_KEYBOARD_FLAGS,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if remaining & (KITTY_KEYBOARD_FLAGS | XTERM_EXTENDED_KEYS | XTERM_OTHER_KEYS_FORMAT) == 0
        && active & ALTERNATE_SCREEN != 0
    {
        match execute!(out, terminal::LeaveAlternateScreen) {
            Ok(()) => remaining &= !ALTERNATE_SCREEN,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if active & TITLE_STACK != 0 {
        match out.write_all(TITLE_STACK_POP).and_then(|()| out.flush()) {
            Ok(()) => remaining &= !TITLE_STACK,
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
