//! Recover descriptor drift without replacing the source buffer or its history.
//! Availability is derived from storage; rendering never changes application state.

use std::io;

use crate::buffer::{BackingFileChanged, BaselinePending};
use crate::config::actions::Action;

use super::App;

pub(super) const UNAVAILABLE: &str = "Paged content unavailable: backing file changed.";

/// Commands that can help the user leave, reload, or inspect the session without
/// reading/editing the unavailable source. Save still uses its descriptor guard.
pub(super) fn action_needs_content(action: Action) -> bool {
    !matches!(
        action,
        Action::Help
            | Action::Quit
            | Action::Interrupt
            | Action::Reload
            | Action::Save
            | Action::SaveAs
            | Action::Open
            | Action::New
            | Action::Close
            | Action::CommandPrompt
            | Action::PreviousBuffer
            | Action::NextBuffer
    )
}

/// The post-operation boundary also covers drift detected after preflight. An
/// edit may already have completed before a later read failed: retain its exact
/// history/dirty state so quit and buffer-close guards still protect it.
pub(super) fn recover(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    result: io::Result<()>,
) -> io::Result<()> {
    match result {
        Err(error) if BackingFileChanged::is(&error) => {
            super::file_state::refresh_dirty(&mut app.file, &*app.buffer);
            super::completion::cancel(app);
            // This notice replaces any confirmation warning. A rejected input
            // or late read failure must not leave an invisible discard armed.
            app.confirmation = super::confirmation::Confirmation::None;
            app.message_error(UNAVAILABLE);
            app.render(out)
        }
        // A paged read refused until the opened revision's ctime tick passes.
        // The content is intact; report it and let the user retry.
        Err(error) if BaselinePending::is(&error) => {
            app.message_error(error.to_string());
            app.render(out)
        }
        result => result,
    }
}
