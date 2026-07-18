//! Terminal panic/restoration unit tests.
//!
//! Real binary PTY smoke coverage lives in tests/pty_smoke.rs, where the test
//! harness can use Cargo's CARGO_BIN_EXE_catomic path.
//!
//! See "Measurement / Test Discipline" in TODO.md.
//!
//! Restoration smokes use in-memory writers and std::panic::catch_unwind to
//! verify terminal modes unwind without double cleanup or broken invariants.

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use crate::terminal::{PanicRestoreGuard, TerminalGuard};

    fn panic_hook_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
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

    #[test]
    fn panic_notice_is_helpful_without_promising_unsaved_work_survived() {
        let notice = crate::terminal::PANIC_NOTICE;

        assert!(notice.contains("Terminal restored"));
        assert!(notice.contains("last explicit save is safe"));
        assert!(!notice.contains("unsaved"));
    }

    #[test]
    fn panic_restoration_pops_keyboard_flags_exactly_once() {
        let _lock = panic_hook_test_lock().lock().unwrap();
        let terminal = TerminalGuard::new();
        terminal
            .enable_output_modes_for_test(&mut Vec::new())
            .unwrap();
        let restored = Arc::new(Mutex::new(Vec::new()));
        let restored_seen = restored.clone();
        let restorer = terminal.restorer();

        {
            let _panic_guard = PanicRestoreGuard::install_with_restore_for_test(move || {
                let _ = restorer.restore(&mut SharedWriter(restored_seen.clone()));
            });
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                panic!("simulated panic with enhanced keyboard flags");
            }));
            assert!(result.is_err());
        }
        drop(terminal);

        let restored = restored.lock().unwrap();
        assert_eq!(count(&restored, b"\x1b[<1u"), 1);
        assert_eq!(count(&restored, b"\x1b[?1049l"), 1);
    }

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn count(bytes: &[u8], needle: &[u8]) -> usize {
        bytes
            .windows(needle.len())
            .filter(|part| *part == needle)
            .count()
    }
}
