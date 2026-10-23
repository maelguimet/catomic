//! Purpose: route keys, paste, and scoped actions to the focused surface and its workers.
//! Owns: the canonical key order between background workers and the focused surface.
//! Must not: edit buffer content, translate keybindings, decode bytes, or start background work.
//! Invariants: paste and mobile reach the surface that receives keys; workers only intercept
//!   their own cancel key, at their place in the key order.

use std::io;

use crate::config::actions::{Action, Scope};
use crossterm::event::KeyEvent;

use super::super::focus::Surface;
use super::super::{
    command_prompt, completion, external_command, help, lint, recovery, replace, search, view, App,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyStop {
    LintWorker,
    CommandWorker,
    GotoWorker,
    CompletionTrigger,
    Focus(Surface),
}

const KEY_ORDER: [KeyStop; 13] = [
    KeyStop::LintWorker,
    KeyStop::Focus(Surface::Help),
    KeyStop::Focus(Surface::RecoveryPreview),
    KeyStop::CommandWorker,
    KeyStop::Focus(Surface::CommandPreview),
    KeyStop::Focus(Surface::Replace),
    KeyStop::Focus(Surface::ReplaceReview),
    KeyStop::Focus(Surface::Search),
    KeyStop::Focus(Surface::Command),
    KeyStop::GotoWorker,
    KeyStop::Focus(Surface::Picker),
    KeyStop::CompletionTrigger,
    KeyStop::Focus(Surface::MarkdownPreview),
];

pub(super) fn handle_raw_key(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    let focus = app.focus.surface();
    for stop in KEY_ORDER {
        let handled = match stop {
            KeyStop::LintWorker => lint::handle_key(app, out, key)?,
            KeyStop::CommandWorker => external_command::handle_worker_key(app, out, key)?,
            KeyStop::GotoWorker => command_prompt::handle_goto_worker_key(app, out, key)?,
            KeyStop::CompletionTrigger if focus == Surface::Editor => {
                completion::handle_trigger_key(app, out, key)?
            }
            KeyStop::CompletionTrigger => false,
            KeyStop::Focus(surface) if surface == focus => handle_focus_key(app, out, focus, key)?,
            KeyStop::Focus(_) => false,
        };
        if handled {
            return Ok(true);
        }
    }
    Ok(false)
}

fn handle_focus_key(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    surface: Surface,
    key: KeyEvent,
) -> io::Result<bool> {
    match surface {
        Surface::Help => help::handle_key(app, out, key),
        Surface::RecoveryPreview => recovery::handle_key(app, out, key),
        Surface::CommandPreview => external_command::handle_preview_key(app, out, key),
        Surface::Replace | Surface::ReplaceReview => replace::handle_key(app, out, key),
        Surface::Search => search::handle_active_key(app, out, key),
        Surface::Command => command_prompt::handle_active_key(app, out, key),
        Surface::Picker => completion::handle_active_key(app, out, key),
        Surface::MarkdownPreview => view::handle_key(app, out, key),
        Surface::Editor | Surface::MobileOverlay => Ok(false),
    }
}

pub(super) fn dispatch_action(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    scope: Scope,
    action: Action,
) -> io::Result<()> {
    let handled = match app.focus.base_surface() {
        Surface::Help => help::dispatch_action(app, out, action)?,
        Surface::Search => search::dispatch_action(app, out, action)?,
        Surface::Replace | Surface::ReplaceReview => replace::dispatch_action(app, out, action)?,
        Surface::Picker => completion::dispatch_action(app, out, action)?,
        Surface::Command => command_prompt::dispatch_action(app, out, action)?,
        Surface::RecoveryPreview => {
            recovery::dispatch_action(app, out, action)? || view::dispatch_action(app, out, action)?
        }
        Surface::CommandPreview => {
            external_command::dispatch_action(app, out, action)?
                || view::dispatch_action(app, out, action)?
        }
        Surface::MarkdownPreview => {
            external_command::dispatch_worker_action(app, out, action)?
                || view::dispatch_action(app, out, action)?
                || view::dispatch_preview_action(app, out, action)?
        }
        Surface::Editor | Surface::MobileOverlay => match scope {
            Scope::Prompt => command_prompt::dispatch_action(app, out, action)?,
            Scope::Preview => {
                external_command::dispatch_worker_action(app, out, action)?
                    || view::dispatch_action(app, out, action)?
            }
            _ => false,
        },
    };
    if !handled {
        app.render(out)?;
    }
    Ok(())
}

pub(super) fn handle_paste(
    app: &mut App,
    out: &mut dyn crate::terminal::TerminalOutput,
    text: &str,
) -> io::Result<bool> {
    match app.focus.surface() {
        Surface::Help => help::handle_paste(app, out),
        Surface::Replace | Surface::ReplaceReview => replace::handle_paste(app, out, text),
        Surface::Search => search::handle_paste(app, out, text),
        Surface::Command => command_prompt::handle_paste(app, out, text),
        Surface::RecoveryPreview => recovery::handle_paste(app, out),
        Surface::CommandPreview => external_command::handle_paste(app, out),
        Surface::MarkdownPreview => view::handle_paste(app, out),
        Surface::Editor | Surface::Picker | Surface::MobileOverlay => Ok(false),
    }
}

#[cfg(test)]
#[path = "surfaces_tests.rs"]
mod tests;
