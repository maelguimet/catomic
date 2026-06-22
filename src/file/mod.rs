//! File I/O, watching, and recovery.
//!
//! - io.rs: read / write (atomic writes implemented)
//! - watcher.rs: notify-based external edit detection (Phase 2)
//! - recovery.rs: .catnap autosave (later, opt-in / safe default)

pub mod io;
pub mod recovery;
pub mod watcher;
