//! Buffer unit and property tests.
//!
//! This is now a small hub. Submodules own focused groups of tests.
//! All are under `buffer::tests::*` so `cargo test buffer::tests::...` works.
//! Shared helpers (if cross-module) live here with pub(super) visibility.
//! Random-edit tests compare production storage against a simple String model.
//!

#[cfg(test)]
mod basic;
#[cfg(test)]
mod edit_parity;
#[cfg(test)]
mod history_position;
#[cfg(test)]
mod model_parity;
#[cfg(test)]
mod range_edit;
#[cfg(test)]
mod storage_parity;
#[cfg(test)]
mod undo_redo;

// (All tests extracted; no more catch-all temp mod.)
