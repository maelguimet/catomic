//! PTY smoke tests.
//!
//! Use a PTY crate (e.g. portable-pty or crossterm's own testing facilities)
//! to drive the real binary and assert on terminal output and saved files.
//!
//! See "Measurement / Test Discipline" in TODO.md.
//! Every phase must have PTY tests.

#[cfg(test)]
pub fn _placeholder_pty_test() {
    // Example future test:
    // let mut child = spawn_catomic_on_tempfile(...);
    // send_keys(&mut child, "hello world\r\n");
    // assert_saved_content(...);
}
