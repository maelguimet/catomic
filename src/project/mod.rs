//! Project mode features.
//!
//! Everything under this module must be gated by `Capabilities`.
//! In Plain mode, **nothing here should be constructed**.
//! See `docs/architecture.md` for the corresponding system boundary.

pub mod diagnostics;
pub mod discovery;
pub mod git;
pub(crate) mod linter;
mod session;

pub(crate) use session::ProjectSession;
