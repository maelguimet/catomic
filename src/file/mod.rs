//! File I/O, watching, and recovery.
//!
//! - identity.rs: same-file buffer identity for existing and missing paths
//! - io.rs: read / write (atomic writes implemented)
//! - size.rs: metadata-only size classification (Phase 2B foundation)
//! - watcher.rs: notify-based external edit detection (Phase 2)
//! - recovery.rs: bounded private .catnap storage and worker

pub(crate) mod identity;
pub mod io;
pub mod recovery;
pub mod size;
pub mod text_format;
pub(crate) mod watch_path;
pub mod watcher;
