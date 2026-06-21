//! Terminal handling: raw mode, alternate screen, input, render, screen model.
//!
//! Philosophy (from TODO):
//! - Use crossterm directly.
//! - Keep rendering dumb and predictable at first.
//! - Only introduce ratatui or similar much later for optional widgets.

pub mod input;
pub mod render;
pub mod screen;

/// Setup raw mode + alternate screen.
/// Must be paired with teardown on all exit paths (including panic).
pub fn setup<W: std::io::Write>(w: &mut W) -> std::io::Result<()> {
    use crossterm::{execute, terminal};
    execute!(w, terminal::EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    // TODO: bracketed paste enable (see "Terminal Realities" in TODO.md)
    Ok(())
}

/// Restore terminal state.
pub fn teardown<W: std::io::Write>(w: &mut W) -> std::io::Result<()> {
    use crossterm::{execute, terminal};
    terminal::disable_raw_mode()?;
    execute!(w, terminal::LeaveAlternateScreen)?;
    Ok(())
}
