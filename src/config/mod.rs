//! Purpose: load small, std-only user configuration with safe defaults.
//! Owns: config path discovery and focused configuration submodules.
//! Must not: construct Project/LLM services, perform network work, or mutate files.
//! Invariants: no config file is required; malformed recognized values are errors;
//!   unknown keys are ignored for forward compatibility.
//! Phase: 2-bk configurable paging through 2-bx automatic reload policy.

pub mod auto_reload;
pub mod big_files;
pub(crate) mod linters;
pub(crate) mod llm;
