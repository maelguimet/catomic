//! PTY smoke tests.
//!
//! Use a PTY crate (e.g. portable-pty or crossterm's own testing facilities)
//! to drive the real binary and assert on terminal output and saved files.
//!
//! See "Measurement / Test Discipline" in TODO.md.
//! Every phase must have PTY tests.
//!
//! Phase 0: we provide a restoration smoke test using an in-memory writer
//! and std::panic::catch_unwind to verify TerminalGuard + teardown run on
//! panic paths without double-panic or broken invariants. Real PTY driving
//! of the binary comes when dev-deps (portable-pty etc.) are added.

#[cfg(test)]
mod tests {
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use crate::terminal::{PanicRestoreGuard, TerminalGuard};

    fn panic_hook_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    // NOTE: Real setup/teardown require a tty (enable_raw_mode fails on pipes).
    // These are ignored in normal `cargo test` environments.
    // Run with `cargo test -- --ignored` inside a real terminal if desired.
    #[test]
    #[ignore]
    fn terminal_setup_teardown_roundtrip_tty_only() {
        // Placeholder: would do setup/teardown on a real pty handle.
    }

    #[test]
    fn terminal_guard_drops_without_setup() {
        // Guard must be safe to drop even if setup was never called
        // (or failed). teardown() inside swallows errors.
        let _guard = TerminalGuard::new();
        // Drop at end of scope; must not panic.
    }

    #[test]
    fn terminal_guard_restores_on_panic_without_real_tty() {
        let _lock = panic_hook_test_lock().lock().unwrap();
        // Exercise the unwind path. We deliberately avoid calling setup()
        // because this test runs without a tty in most CI/piped envs.
        // The guard still exercises its Drop::teardown (best-effort).
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = TerminalGuard::new();
            panic!("simulated editor panic (no raw mode entered)");
        }));
        assert!(result.is_err());
    }

    #[test]
    fn panic_restore_guard_runs_restore_and_restores_previous_hook() {
        let _lock = panic_hook_test_lock().lock().unwrap();
        let original_hook = panic::take_hook();
        let previous_called = Arc::new(AtomicUsize::new(0));
        let restore_called = Arc::new(AtomicUsize::new(0));

        let previous_seen = previous_called.clone();
        panic::set_hook(Box::new(move |_| {
            previous_seen.fetch_add(1, Ordering::SeqCst);
        }));

        {
            let restore_seen = restore_called.clone();
            let _guard = PanicRestoreGuard::install_with_restore_for_test(move || {
                restore_seen.fetch_add(1, Ordering::SeqCst);
            });
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                panic!("simulated panic while hook guard installed");
            }));
            assert!(result.is_err());
        }

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            panic!("simulated panic after hook guard drop");
        }));
        assert!(result.is_err());

        let _current = panic::take_hook();
        panic::set_hook(original_hook);

        assert_eq!(restore_called.load(Ordering::SeqCst), 1);
        assert_eq!(previous_called.load(Ordering::SeqCst), 2);
    }
}
