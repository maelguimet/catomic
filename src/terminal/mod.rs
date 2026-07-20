//! Purpose: own terminal sessions, input protocol setup, and panic-safe restoration.
//! Owns: terminal mode guards, signal integration, and the user-facing panic notice.
//! Must not: interpret editor commands, mutate App/Buffer state, render content, or network.
//! Invariants: every enabled terminal mode has a best-effort inverse on all exit paths.
//! Phase: post-v0.1 enhanced keyboard reporting and panic-safe restoration.

pub(crate) mod cursor_style;
pub mod render;
pub mod screen;
mod session;
mod signal;
mod title;

pub(crate) use session::TerminalGuard;
pub(crate) use signal::{
    install_process_handlers, request_interrupt, take_resize_pending, termination_signal,
};

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use session::TerminalRestorer;

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

pub(crate) const PANIC_NOTICE: &str =
    "catomic: the cat knocked over the editor. Terminal restored; your last explicit save is safe.";

/// Installs a panic hook that restores terminal state before chaining to the
/// previously installed hook. Restores the previous hook when dropped.
pub(crate) struct PanicRestoreGuard {
    previous: Arc<Mutex<Option<PanicHook>>>,
}

impl PanicRestoreGuard {
    pub(crate) fn install(restorer: TerminalRestorer) -> Self {
        Self::install_with_restore(move || restorer.restore_stdout())
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
            let _ = writeln!(io::stderr().lock(), "{PANIC_NOTICE}");
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
