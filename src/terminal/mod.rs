//! Purpose: own terminal sessions, input protocol setup, and panic-safe restoration.
//! Owns: terminal mode guards, signal integration, and the user-facing panic notice.
//! Must not: interpret editor commands, mutate App/Buffer state, render content, or network.
//! Invariants: every enabled terminal mode has a best-effort inverse on all exit paths.

pub(crate) mod cursor_style;
mod output;
pub mod render;
pub mod screen;
mod session;
mod signal;
mod title;

pub(crate) use output::{RuntimeOutput, TerminalOutput};
pub(crate) use session::{
    require_editor_terminal, require_piped_input_terminal, settle_input_after_quit, MouseTracking,
    TerminalGuard,
};
pub(crate) use signal::{
    install_process_handlers, request_interrupt, take_resize_pending, termination_signal,
};

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use session::TerminalRestorer;

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

pub(crate) const PANIC_NOTICE: &str =
    "catomic: the cat knocked over the editor. Terminal restored; your last explicit save is safe.";

type RestoreFn = Arc<dyn Fn() + Sync + Send + 'static>;

/// Installs a panic hook that restores terminal state before chaining to the
/// previously installed hook. Restores the previous hook when dropped.
///
/// The process-wide hook also runs for background worker threads. Their
/// callers see the failure through a join handle or a disconnected channel
/// (or, for some workers, as a result that never arrives) while the session
/// keeps running, so a worker panic must not restore the terminal or write to
/// stderr under the live editor screen. The first worker panic is kept
/// and reported once the terminal has been restored.
pub(crate) struct PanicRestoreGuard {
    previous: Arc<Mutex<Option<PanicHook>>>,
    restore: RestoreFn,
    worker_panic: Arc<Mutex<Option<String>>>,
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
        let restore: RestoreFn = Arc::new(restore);
        let previous = Arc::new(Mutex::new(Some(std::panic::take_hook())));
        let worker_panic = Arc::new(Mutex::new(None));
        let hook_previous = previous.clone();
        let hook_restore = restore.clone();
        let hook_worker_panic = worker_panic.clone();
        let session_thread = std::thread::current().id();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            if thread.id() != session_thread {
                let mut recorded = lock_ignoring_poison(&hook_worker_panic);
                if recorded.is_none() {
                    let name = thread.name().unwrap_or("<unnamed>");
                    *recorded = Some(format!("thread '{name}' {info}"));
                }
                return;
            }
            hook_restore();
            let _ = writeln!(io::stderr().lock(), "{PANIC_NOTICE}");
            if let Some(prev) = lock_ignoring_poison(&hook_previous).as_ref() {
                prev(info);
            }
        }));
        Self {
            previous,
            restore,
            worker_panic,
        }
    }

    #[cfg(test)]
    pub(crate) fn worker_panic_for_test(&self) -> Option<String> {
        lock_ignoring_poison(&self.worker_panic).clone()
    }
}

impl Drop for PanicRestoreGuard {
    fn drop(&mut self) {
        let _installed = std::panic::take_hook();
        if let Some(previous) = lock_ignoring_poison(&self.previous).take() {
            std::panic::set_hook(previous);
        }
        if let Some(message) = lock_ignoring_poison(&self.worker_panic).take() {
            // Restoration is idempotent; it ensures the report lands on the
            // shell's screen even when the session ends through an error path.
            (self.restore)();
            let _ = writeln!(
                io::stderr().lock(),
                "catomic: a background task failed during the session: {message}"
            );
        }
    }
}

/// A panic hook must never panic itself; a poisoned lock still holds valid data.
fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
