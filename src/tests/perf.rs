//! Purpose: this file must act as the tiny hub for the split perf harness.
//!   It declares the helpers/default/manual submodules (no logic here).
//! Owns: module declarations only. All behavior lives in siblings (see their headers).
//! Must not: contain test bodies, generators, or timing logic.
//! Invariants: same test names and discovery via `cargo test tests::perf` and bare fn names;
//!   split produces identical observable test behavior (first split commit has zero changes).

//! Re-exports are not needed; tests are discovered by name regardless of nesting.

#[cfg(test)]
#[path = "perf_helpers.rs"]
mod helpers;

#[cfg(test)]
pub(crate) use helpers::count_thread_allocations;

#[cfg(test)]
#[path = "perf_default.rs"]
mod default;

#[cfg(test)]
#[path = "perf_manual.rs"]
mod manual;

#[cfg(test)]
#[path = "perf_manual_line.rs"]
mod manual_line;

#[cfg(test)]
#[path = "perf_cursor.rs"]
mod cursor;

#[cfg(test)]
#[path = "perf_search.rs"]
mod search;

#[cfg(test)]
#[path = "perf_byte_scans.rs"]
mod byte_scans;

#[cfg(test)]
#[path = "perf_render.rs"]
mod render;

#[cfg(test)]
#[path = "perf_editing.rs"]
mod editing;

#[cfg(test)]
#[path = "perf_paged.rs"]
mod paged;

#[cfg(test)]
#[path = "perf_extensibility.rs"]
mod extensibility;

#[cfg(test)]
#[path = "perf_recovery.rs"]
mod recovery;

#[cfg(test)]
#[path = "perf_history.rs"]
mod history;
