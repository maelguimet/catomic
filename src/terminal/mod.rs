//! Terminal handling: raw mode, alternate screen, input, render, screen model.
//!
//! Philosophy (from TODO):
//! - Use crossterm directly.
//! - Keep rendering dumb and predictable at first.
//! - Only introduce ratatui or similar much later for optional widgets.

pub mod input;
pub mod render;
pub mod screen;

use std::io::{self, Write};

/// Setup raw mode + alternate screen.
/// Must be paired with teardown on all exit paths (including panic).
pub fn setup<W: Write>(w: &mut W) -> io::Result<()> {
    use crossterm::{execute, terminal};
    execute!(w, terminal::EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    // TODO: bracketed paste enable (see "Terminal Realities" in TODO.md)
    Ok(())
}

/// Restore terminal state.
/// Safe to call multiple times / when not in raw mode.
pub fn teardown<W: Write>(w: &mut W) -> io::Result<()> {
    use crossterm::{execute, terminal};
    // Ignore errors: we are best-effort during panic paths.
    let _ = terminal::disable_raw_mode();
    let _ = execute!(w, terminal::LeaveAlternateScreen);
    Ok(())
}

/// Guard that restores the terminal on drop (normal exit or panic unwind).
/// Install early after setup. Dropping it guarantees best-effort restore.
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn new() -> Self {
        Self
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort restore even if we don't have the original writer.
        // Fresh stdout is sufficient for disable + leave alt screen.
        let mut out = io::stdout();
        let _ = teardown(&mut out);
    }
}
