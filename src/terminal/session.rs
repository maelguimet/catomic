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
mod tests {
    use super::*;

    #[test]
    fn setup_and_repeated_restore_push_and_pop_keyboard_flags_once() {
        let guard = TerminalGuard::new();
        let mut output = Vec::new();

        guard.enable_output_modes(&mut output).unwrap();
        guard.restore(&mut output).unwrap();
        guard.restore(&mut output).unwrap();

        assert_eq!(count(&output, b"\x1b[>9u"), 1);
        assert_eq!(count(&output, b"\x1b[<1u"), 1);
        assert_eq!(count(&output, b"\x1b[0 q"), 1);
        assert_eq!(count(&output, b"\x1b]112\x07"), 1);
        assert!(position(&output, b"\x1b[?1049h") < position(&output, b"\x1b[>9u"));
        assert!(position(&output, b"\x1b[<1u") < position(&output, b"\x1b[?1049l"));
    }

    #[test]
    fn setup_error_before_keyboard_push_leaves_screen_without_pop() {
        let guard = TerminalGuard::new();
        let mut failing = FailAfter::new(b"\x1b[?1049h".len());

        assert!(guard.enable_output_modes(&mut failing).is_err());
        let mut restored = Vec::new();
        guard.restore(&mut restored).unwrap();

        assert_eq!(count(&restored, b"\x1b[<1u"), 0);
        assert_eq!(count(&restored, b"\x1b[?1049l"), 1);
    }

    #[test]
    fn setup_error_after_keyboard_push_still_pops_before_leaving_screen() {
        let guard = TerminalGuard::new();
        let setup_prefix = b"\x1b[?1049h\x1b[>9u";
        let mut failing = FailAfter::new(setup_prefix.len());

        assert!(guard.enable_output_modes(&mut failing).is_err());
        let mut restored = Vec::new();
        guard.restore(&mut restored).unwrap();

        assert_eq!(count(&restored, b"\x1b[<1u"), 1);
        assert!(position(&restored, b"\x1b[<1u") < position(&restored, b"\x1b[?1049l"));
    }

    #[test]
    fn teardown_error_before_pop_does_not_cause_a_duplicate_pop() {
        let guard = TerminalGuard::new();
        guard.enable_output_modes(&mut Vec::new()).unwrap();
        let mut failing = FailOnceOn::new(b"\x1b[?1006l");

        assert!(guard.restore(&mut failing).is_err());
        assert_eq!(count(&failing.output, b"\x1b[<1u"), 1);
        assert_eq!(count(&failing.output, b"\x1b[?1049l"), 1);

        let mut repeated = Vec::new();
        guard.restore(&mut repeated).unwrap();
        assert!(repeated.is_empty());
    }

    #[test]
    fn failed_pop_keeps_alternate_screen_active_for_a_retry() {
        let guard = TerminalGuard::new();
        guard.enable_output_modes(&mut Vec::new()).unwrap();
        let mut failing = FailOnceOn::new(b"\x1b[<1u");

        assert!(guard.restore(&mut failing).is_err());
        assert_eq!(count(&failing.output, b"\x1b[?1049l"), 0);

        let mut retried = Vec::new();
        guard.restore(&mut retried).unwrap();
        assert_eq!(count(&retried, b"\x1b[<1u"), 1);
        assert_eq!(count(&retried, b"\x1b[?1049l"), 1);
    }

    fn count(bytes: &[u8], needle: &[u8]) -> usize {
        bytes
            .windows(needle.len())
            .filter(|part| *part == needle)
            .count()
    }

    fn position(bytes: &[u8], needle: &[u8]) -> usize {
        bytes
            .windows(needle.len())
            .position(|part| part == needle)
            .expect("terminal sequence")
    }

    struct FailAfter {
        accepted: usize,
        limit: usize,
    }

    impl FailAfter {
        fn new(limit: usize) -> Self {
            Self { accepted: 0, limit }
        }
    }

    impl Write for FailAfter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.accepted.saturating_add(bytes.len()) > self.limit {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "test writer failed",
                ));
            }
            self.accepted += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailOnceOn {
        needle: &'static [u8],
        failed: bool,
        output: Vec<u8>,
    }

    impl FailOnceOn {
        fn new(needle: &'static [u8]) -> Self {
            Self {
                needle,
                failed: false,
                output: Vec::new(),
            }
        }
    }

    impl Write for FailOnceOn {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !self.failed
                && bytes
                    .windows(self.needle.len())
                    .any(|part| part == self.needle)
            {
                self.failed = true;
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "test writer failed",
                ));
            }
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
