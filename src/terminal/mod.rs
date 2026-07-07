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
use std::sync::{Arc, Mutex};

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

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

/// Installs a panic hook that restores terminal state before chaining to the
/// previously installed hook. Restores the previous hook when dropped.
pub(crate) struct PanicRestoreGuard {
    previous: Arc<Mutex<Option<PanicHook>>>,
}

impl PanicRestoreGuard {
    pub(crate) fn install() -> Self {
        Self::install_with_restore(|| {
            let _ = teardown(&mut io::stdout());
        })
    }

    #[cfg(test)]
    pub(crate) fn install_with_restore_for_test(
        restore: impl Fn() + Sync + Send + 'static,
    ) -> Self {
        Self::install_with_restore(restore)
    }

    fn install_with_restore(restore: impl Fn() + Sync + Send + 'static) -> Self {
        let previous = Arc::new(Mutex::new(Some(std::panic::take_hook())));
        let hook_previous = previous.clone();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            if let Some(prev) = hook_previous.lock().expect("panic hook mutex").as_ref() {
                prev(info);
            }
        }));
        Self { previous }
    }
}

impl Drop for PanicRestoreGuard {
    fn drop(&mut self) {
        let _installed = std::panic::take_hook();
        if let Some(previous) = self.previous.lock().expect("panic hook mutex").take() {
            std::panic::set_hook(previous);
        }
    }
}
