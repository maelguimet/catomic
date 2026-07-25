//! Purpose: group transient read-only surfaces owned by the application shell.
//! Owns: optional help state.
//! Must not: construct workers, processes, or network state.
//! Invariants: every surface is absent at startup and created only by its explicit action.

use super::help;

#[derive(Default)]
pub(crate) struct SurfaceState {
    pub(crate) help: Option<help::HelpView>,
}
