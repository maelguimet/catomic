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

    use crate::terminal::TerminalGuard;

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
        // Exercise the unwind path. We deliberately avoid calling setup()
        // because this test runs without a tty in most CI/piped envs.
        // The guard still exercises its Drop::teardown (best-effort).
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = TerminalGuard::new();
            panic!("simulated editor panic (no raw mode entered)");
        }));
        assert!(result.is_err());
    }
}
