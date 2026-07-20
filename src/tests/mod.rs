//! Test helpers and infrastructure that live with the source.
//!
//! - pty.rs: PTY smoke tests (launch editor, send keys, assert screen/save)
//! - golden.rs: golden file tests (edit sequence → exact output file)
//! - perf.rs: benchmarks / perf targets (10MB smooth, etc.)
//!
//! Real tests will also live in `tests/` at the crate root for integration
//! and in `#[cfg(test)]` modules next to the code.

pub mod golden;
pub mod golden_phase4;
pub mod golden_phase6;
pub mod perf;
pub mod pty;
