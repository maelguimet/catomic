//! Purpose: make key and paste surface precedence explicit and independently testable.
//! Owns: ordered dispatch across active prompts, previews, pickers, and editor surfaces.
//! Must not: edit buffer content, translate keybindings, decode bytes, or start background work.
//! Invariants: active surfaces precede editor actions.

use std::io;

use crate::config::actions::{Action, Scope};
use crossterm::event::KeyEvent;

use super::super::{
    command_prompt, completion, external_command, help, lint, recovery, replace, search, view, App,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawKeySurface {
    Lint,
    Help,
    Recovery,
    ExternalCommand,
    Replace,
    Search,
    CommandPrompt,
    Completion,
    MarkdownPreview,
}

const RAW_KEY_PRECEDENCE: [RawKeySurface; 9] = [
    RawKeySurface::Lint,
    RawKeySurface::Help,
    RawKeySurface::Recovery,
    RawKeySurface::ExternalCommand,
    RawKeySurface::Replace,
    RawKeySurface::Search,
    RawKeySurface::CommandPrompt,
    RawKeySurface::Completion,
    RawKeySurface::MarkdownPreview,
];

pub(super) fn handle_raw_key(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    for surface in RAW_KEY_PRECEDENCE {
        if handle_raw_key_for(surface, app, out, key)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn handle_raw_key_for(
    surface: RawKeySurface,
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    match surface {
        RawKeySurface::Lint => lint::handle_key(app, out, key),
        RawKeySurface::Help => help::handle_key(app, out, key),
        RawKeySurface::Recovery => recovery::handle_key(app, out, key),
        RawKeySurface::ExternalCommand => external_command::handle_key(app, out, key),
        RawKeySurface::Replace => replace::handle_key(app, out, key),
        RawKeySurface::Search => search::handle_active_key(app, out, key),
        RawKeySurface::CommandPrompt => command_prompt::handle_active_key(app, out, key),
        RawKeySurface::Completion => completion::handle_key(app, out, key),
        RawKeySurface::MarkdownPreview if view::is_preview(app) => view::handle_key(app, out, key),
        RawKeySurface::MarkdownPreview => Ok(false),
    }
}

pub(super) fn dispatch_action(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    scope: Scope,
    action: Action,
) -> io::Result<()> {
    let handled = match scope {
        Scope::Help => help::dispatch_action(app, out, action)?,
        Scope::Search => search::dispatch_action(app, out, action)?,
        Scope::Completion => completion::dispatch_action(app, out, action)?,
        Scope::Prompt => {
            replace::dispatch_action(app, out, action)?
                || command_prompt::dispatch_action(app, out, action)?
        }
        Scope::Preview => {
            recovery::dispatch_action(app, out, action)?
                || external_command::dispatch_action(app, out, action)?
                || view::dispatch_action(app, out, action)?
                || view::dispatch_preview_action(app, out, action)?
        }
        Scope::Global | Scope::Editor => false,
    };
    if !handled {
        app.render(out)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteSurface {
    Help,
    Replace,
    Search,
    CommandPrompt,
    Recovery,
    ExternalCommand,
    MarkdownPreview,
}

const PASTE_PRECEDENCE: [PasteSurface; 7] = [
    PasteSurface::Help,
    PasteSurface::Replace,
    PasteSurface::Search,
    PasteSurface::CommandPrompt,
    PasteSurface::Recovery,
    PasteSurface::ExternalCommand,
    PasteSurface::MarkdownPreview,
];

pub(super) fn handle_paste(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    text: &str,
) -> io::Result<bool> {
    for surface in PASTE_PRECEDENCE {
        let handled = match surface {
            PasteSurface::Help => help::handle_paste(app, out)?,
            PasteSurface::Replace => replace::handle_paste(app, out, text)?,
            PasteSurface::Search => search::handle_paste(app, out, text)?,
            PasteSurface::CommandPrompt => command_prompt::handle_paste(app, out, text)?,
            PasteSurface::Recovery => recovery::handle_paste(app, out)?,
            PasteSurface::ExternalCommand => external_command::handle_paste(app, out)?,
            PasteSurface::MarkdownPreview => view::handle_paste(app, out)?,
        };
        if handled {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_contracts_are_named_and_locked() {
        assert_eq!(RAW_KEY_PRECEDENCE[0], RawKeySurface::Lint);
        assert_eq!(RAW_KEY_PRECEDENCE[1], RawKeySurface::Help);
        assert_eq!(RAW_KEY_PRECEDENCE[4], RawKeySurface::Replace);
        assert_eq!(RAW_KEY_PRECEDENCE[5], RawKeySurface::Search);
        assert_eq!(RAW_KEY_PRECEDENCE[6], RawKeySurface::CommandPrompt);
        assert_eq!(RAW_KEY_PRECEDENCE[8], RawKeySurface::MarkdownPreview);
        assert_eq!(PASTE_PRECEDENCE[0], PasteSurface::Help);
        assert_eq!(PASTE_PRECEDENCE[1], PasteSurface::Replace);
        assert_eq!(PASTE_PRECEDENCE[2], PasteSurface::Search);
        assert_eq!(PASTE_PRECEDENCE[3], PasteSurface::CommandPrompt);
        assert_eq!(PASTE_PRECEDENCE[6], PasteSurface::MarkdownPreview);
    }
}
