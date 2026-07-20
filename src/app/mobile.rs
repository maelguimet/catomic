//! Purpose: make essential editor actions reachable through touch and soft keyboards.
//! Owns: mobile UI enablement, contextual action dispatch, and reserved-row hit testing.
//! Must not: duplicate editor commands, inspect file internals, start services, or write files.
//! Invariants: actions reuse normalized key paths; status/action touches never reach content.
//! Phase: Android/Termux mobile support.

use std::io::{self, Write};

use crate::config::actions::{Action, Scope};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

mod actions;
mod bar;
mod overlay;

use actions::MenuAction;
use bar::{ActionBar, BarAction, Surface};
pub(crate) use overlay::MobileUiState;

pub(crate) fn configure(app: &mut super::App, enabled: bool) {
    app.mobile.enabled = enabled;
    app.screen.set_action_bar(enabled);
    if !enabled {
        overlay::close(app);
    }
}

pub(crate) fn is_enabled(app: &super::App) -> bool {
    app.mobile.enabled
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    overlay::is_viewing(app)
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn crate::buffer::Buffer> {
    overlay::display_buffer(app)
}

fn action_bar(app: &super::App) -> Option<ActionBar> {
    app.mobile
        .enabled
        .then(|| bar::build(active_surface(app), app.screen.width as usize))
}

pub(crate) fn action_bar_text(app: &super::App) -> Option<String> {
    action_bar(app).map(|bar| bar.text)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if !overlay::is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    match key.code {
        KeyCode::Esc => {
            overlay::close(app);
            app.reveal_cursor();
            app.render(out)?;
        }
        KeyCode::Enter if overlay::is_menu(app) => {
            if let Some(action) = overlay::selected_action(app) {
                execute_menu_action(app, out, action)?;
            }
        }
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Home
        | KeyCode::End => {
            overlay::move_cursor(app, key.code);
            app.reveal_cursor();
            app.render(out)?;
        }
        _ => {
            app.message = Some("Mobile overlay is read-only; Back returns.".to_string());
            app.render(out)?;
        }
    }
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !overlay::is_viewing(app) {
        return Ok(false);
    }
    app.message = Some("Mobile overlay is read-only; Back returns.".to_string());
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_mouse(
    app: &mut super::App,
    out: &mut dyn Write,
    event: MouseEvent,
) -> io::Result<bool> {
    if !app.mobile.enabled {
        return Ok(false);
    }
    let row = event.row as usize;
    let height = app.screen.height as usize;
    if row >= app.screen.visible_height() {
        if row == height.saturating_sub(1)
            && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
        {
            if let Some(action) =
                action_bar(app).and_then(|bar| bar.action_at(event.column as usize))
            {
                dispatch_bar_action(app, out, action)?;
            }
        }
        return Ok(true);
    }
    if !overlay::is_viewing(app) {
        return Ok(false);
    }
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) if overlay::is_menu(app) => {
            if let Some(action) = overlay::action_at_visible_row(app, row) {
                execute_menu_action(app, out, action)?;
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let document_row = app.screen.scroll_top.saturating_add(row);
            overlay::set_cursor_row(app, document_row);
            app.reveal_cursor();
            app.render(out)?;
        }
        MouseEventKind::ScrollUp => move_overlay(app, out, KeyCode::Up)?,
        MouseEventKind::ScrollDown => move_overlay(app, out, KeyCode::Down)?,
        _ => {}
    }
    Ok(true)
}

fn dispatch_bar_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: BarAction,
) -> io::Result<()> {
    match action {
        BarAction::Menu => {
            overlay::open_menu(app);
            app.render(out)
        }
        BarAction::Info => {
            let message = app
                .message
                .clone()
                .unwrap_or_else(|| "No details.".to_string());
            overlay::open_notice(app, &message);
            app.render(out)
        }
        BarAction::Back if overlay::close(app) => {
            app.reveal_cursor();
            app.render(out)
        }
        BarAction::Accept if overlay::is_menu(app) => match overlay::selected_action(app) {
            Some(action) => execute_menu_action(app, out, action),
            None => app.render(out),
        },
        BarAction::Cancel if super::selection::is_touch_selecting(app) => {
            super::selection::cancel_touch_selection(app);
            app.message = None;
            app.render(out)
        }
        BarAction::Cancel | BarAction::Back => dispatch_surface_action(app, out, false),
        BarAction::Accept => dispatch_surface_action(app, out, true),
        BarAction::Up => super::input::dispatch_action(app, out, Action::MoveUp),
        BarAction::Down => super::input::dispatch_action(app, out, Action::MoveDown),
        BarAction::PageUp => super::input::dispatch_action(app, out, Action::ViewportUp),
        BarAction::PageDown => super::input::dispatch_action(app, out, Action::ViewportDown),
        BarAction::Save => super::input::dispatch_action(app, out, Action::Save),
        BarAction::Undo => super::input::dispatch_action(app, out, Action::Undo),
        BarAction::Copy => super::input::dispatch_action(app, out, Action::Copy),
        BarAction::Cut => super::input::dispatch_action(app, out, Action::Cut),
    }
}

fn execute_menu_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: MenuAction,
) -> io::Result<()> {
    overlay::close(app);
    match action {
        MenuAction::SelectStart => {
            super::selection::begin_touch_selection(app);
            app.message = Some("Selection start marked. Tap the other end; Cancel aborts.".into());
            app.render(out)
        }
        MenuAction::ScrollUp => super::viewport::scroll_view(app, out, -3),
        MenuAction::ScrollDown => super::viewport::scroll_view(app, out, 3),
        MenuAction::Dispatch(action) => super::input::dispatch_action(app, out, action),
    }
}

fn move_overlay(app: &mut super::App, out: &mut dyn Write, code: KeyCode) -> io::Result<()> {
    overlay::move_cursor(app, code);
    app.reveal_cursor();
    app.render(out)
}

fn dispatch_surface_action(
    app: &mut super::App,
    out: &mut dyn Write,
    accept: bool,
) -> io::Result<()> {
    let action = match (super::input::active_scope(app), accept) {
        (Scope::Prompt, true) => Action::PromptSubmit,
        (Scope::Prompt, false) => Action::PromptCancel,
        (Scope::Search, true) => Action::SearchNext,
        (Scope::Search, false) => Action::SearchCancel,
        (Scope::Completion, true) => Action::CompletionAccept,
        (Scope::Completion, false) => Action::CompletionCancel,
        (Scope::Preview, true) => Action::PreviewAccept,
        (Scope::Preview, false) => Action::PreviewCancel,
        (Scope::Picker, true) => Action::PickerAccept,
        (Scope::Picker, false) => Action::PickerCancel,
        (Scope::Help, false) => Action::HelpClose,
        (Scope::Editor, true) => Action::InsertNewline,
        _ => return app.render(out),
    };
    super::input::dispatch_action(app, out, action)
}

fn active_surface(app: &super::App) -> Surface {
    if overlay::is_menu(app) {
        return Surface::Menu;
    }
    if overlay::is_viewing(app) {
        return Surface::Notice;
    }
    if super::selection::is_touch_selecting(app) {
        return Surface::TouchSelection;
    }
    if app.pending_llm_request.is_some()
        || super::repo_llm::blocks_editing_input(app)
        || super::autocomplete::is_viewing(app)
        || super::llm_preview::is_viewing(app)
        || super::model_picker::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
        || super::project_files::is_viewing(app)
    {
        return Surface::Confirmation;
    }
    if super::search::is_active(app)
        || super::replace::is_active(app)
        || super::command_prompt::is_active(app)
        || super::completion::is_active(app)
    {
        return Surface::Prompt;
    }
    if super::help::is_viewing(app)
        || super::llm_answer::is_viewing(app)
        || super::lint::is_viewing(app)
        || super::view::is_preview(app)
    {
        return Surface::ReadOnly;
    }
    if app.llm_task.is_some()
        || super::repo_llm::is_active(app)
        || super::external_command::is_running(app)
    {
        return Surface::Running;
    }
    if app.pending_save_conflict.is_some() || app.pending_reload.is_some() {
        return Surface::Message;
    }
    if app.selection.active().is_some() {
        Surface::Selection
    } else if app.message.is_some() {
        Surface::Message
    } else {
        Surface::Normal
    }
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
#[path = "mobile/tests.rs"]
mod tests;
