//! Purpose: characterize partial setup, paired keyboard flags, and retry-safe restoration.
//! Owns: in-memory failure writers and terminal session lifecycle assertions.
//! Must not: require a real terminal, mutate editor state, or test input decoding.
//! Invariants: enhanced keyboard states are reset once and before alternate-screen exit.
//! Phase: post-v0.1 terminal keyboard compatibility.

use super::*;

fn tmux_guard() -> TerminalGuard {
    TerminalGuard::with_xterm_extended_keys(true)
}

#[test]
fn setup_and_repeated_restore_push_and_pop_keyboard_flags_once() {
    let guard = tmux_guard();
    let mut output = Vec::new();

    guard.enable_output_modes(&mut output).unwrap();
    guard.restore(&mut output).unwrap();
    guard.restore(&mut output).unwrap();

    assert_eq!(count(&output, b"\x1b[>1u"), 1);
    assert_eq!(count(&output, b"\x1b[>4;2m"), 1);
    assert_eq!(count(&output, b"\x1b[>4;0m"), 1);
    assert_eq!(count(&output, b"\x1b[<1u"), 1);
    assert_eq!(count(&output, b"\x1b[0 q"), 1);
    assert_eq!(count(&output, b"\x1b]112\x07"), 1);
    assert!(position(&output, b"\x1b[?1049h") < position(&output, b"\x1b[>1u"));
    assert!(position(&output, b"\x1b[>4;0m") < position(&output, b"\x1b[<1u"));
    assert!(position(&output, b"\x1b[<1u") < position(&output, b"\x1b[?1049l"));
}

#[test]
fn setup_error_before_keyboard_push_leaves_screen_without_pop() {
    let guard = tmux_guard();
    let mut failing = FailAfter::new(b"\x1b[?1049h".len());

    assert!(guard.enable_output_modes(&mut failing).is_err());
    let mut restored = Vec::new();
    guard.restore(&mut restored).unwrap();

    assert_eq!(count(&restored, b"\x1b[<1u"), 0);
    assert_eq!(count(&restored, b"\x1b[?1049l"), 1);
}

#[test]
fn setup_error_after_both_keyboard_modes_resets_before_leaving_screen() {
    let guard = tmux_guard();
    let setup_prefix = b"\x1b[?1049h\x1b[>1u\x1b[>4;2m";
    let mut failing = FailAfter::new(setup_prefix.len());

    assert!(guard.enable_output_modes(&mut failing).is_err());
    let mut restored = Vec::new();
    guard.restore(&mut restored).unwrap();

    assert_eq!(count(&restored, b"\x1b[>4;0m"), 1);
    assert_eq!(count(&restored, b"\x1b[<1u"), 1);
    assert!(position(&restored, b"\x1b[>4;0m") < position(&restored, b"\x1b[<1u"));
    assert!(position(&restored, b"\x1b[<1u") < position(&restored, b"\x1b[?1049l"));
}

#[test]
fn setup_error_during_xterm_enable_still_pops_kitty_flags() {
    let guard = tmux_guard();
    let setup_prefix = b"\x1b[?1049h\x1b[>1u";
    let mut failing = FailAfter::new(setup_prefix.len());

    assert!(guard.enable_output_modes(&mut failing).is_err());
    let mut restored = Vec::new();
    guard.restore(&mut restored).unwrap();

    assert_eq!(count(&restored, b"\x1b[>4;0m"), 0);
    assert_eq!(count(&restored, b"\x1b[<1u"), 1);
    assert_eq!(count(&restored, b"\x1b[?1049l"), 1);
}

#[test]
fn teardown_error_before_pop_does_not_cause_a_duplicate_pop() {
    let guard = tmux_guard();
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
    let guard = tmux_guard();
    guard.enable_output_modes(&mut Vec::new()).unwrap();
    let mut failing = FailOnceOn::new(b"\x1b[<1u");

    assert!(guard.restore(&mut failing).is_err());
    assert_eq!(count(&failing.output, b"\x1b[?1049l"), 0);

    let mut retried = Vec::new();
    guard.restore(&mut retried).unwrap();
    assert_eq!(count(&retried, b"\x1b[<1u"), 1);
    assert_eq!(count(&retried, b"\x1b[?1049l"), 1);
}

#[test]
fn failed_xterm_reset_retries_without_duplicate_kitty_pop() {
    let guard = tmux_guard();
    guard.enable_output_modes(&mut Vec::new()).unwrap();
    let mut failing = FailOnceOn::new(b"\x1b[>4;0m");

    assert!(guard.restore(&mut failing).is_err());
    assert_eq!(count(&failing.output, b"\x1b[<1u"), 1);
    assert_eq!(count(&failing.output, b"\x1b[?1049l"), 0);

    let mut retried = Vec::new();
    guard.restore(&mut retried).unwrap();
    assert_eq!(count(&retried, b"\x1b[>4;0m"), 1);
    assert_eq!(count(&retried, b"\x1b[<1u"), 0);
    assert_eq!(count(&retried, b"\x1b[?1049l"), 1);
}

#[test]
fn direct_terminal_does_not_enable_xterm_modified_keys() {
    let guard = TerminalGuard::with_xterm_extended_keys(false);
    let mut output = Vec::new();

    guard.enable_output_modes(&mut output).unwrap();
    guard.restore(&mut output).unwrap();

    assert_eq!(count(&output, b"\x1b[>1u"), 1);
    assert_eq!(count(&output, b"\x1b[>4;2m"), 0);
    assert_eq!(count(&output, b"\x1b[>4;0m"), 0);
    assert_eq!(count(&output, b"\x1b[<1u"), 1);
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
