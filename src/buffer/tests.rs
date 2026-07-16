//! Buffer tests (unit + property).
//!
//! Golden tests and property-based tests live here or under src/tests/.
//!
//! Phase 0: basic insert/delete/newline/save roundtrips.
//! Phase 1A+: property tests that random edits on the real impl match a dumb
//! String model. This is non-negotiable.

//! Buffer tests (unit + property). Split in Phase 2-k for size (<800 lines).
//!
//! This is now a small hub. Submodules own focused groups of tests.
//! All are under `buffer::tests::*` so `cargo test buffer::tests::...` works.
//! Shared helpers (if cross-module) live here with pub(super) visibility.
//!
//! Phase: 2-k narrow cleanup (no behavior or API change).

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
