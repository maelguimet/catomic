//! Project mode features.
//!
//! Everything under this module must be gated by `Capabilities`.
//! In Plain mode, **nothing here should be constructed**.
//!
//! See TODO.md "Product Modes", "Capabilities", and Phase 5/6.

pub mod diagnostics;
pub mod discovery;
pub mod git;
pub(crate) mod linter;
mod session;

pub(crate) use session::ProjectSession;
