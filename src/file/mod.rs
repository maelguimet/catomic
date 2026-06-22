//! File I/O, watching, and recovery.
//!
//! - io.rs: read / write (atomic writes implemented)
//! - size.rs: metadata-only size classification (Phase 2B foundation)
//! - watcher.rs: notify-based external edit detection (Phase 2)
//! - recovery.rs: .catnap autosave (later, opt-in / safe default)

pub mod io;
pub mod recovery;
pub mod size;
pub(crate) mod watch_path;
pub mod watcher;
