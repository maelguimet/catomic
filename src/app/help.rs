//! Purpose: present the built-in key and command reference as read-only text.
//! Owns: help view lifetime, navigation, and source viewport restoration.
//! Must not: mutate source/history, read configuration, spawn work, or access network.
//! Invariants: Ctrl+H/F1 toggle the view; Escape closes it; all content is read-only.
//! Phase: post-v0.1 core usability.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::help_catalog::{self, EditorAction};

pub(crate) struct HelpView {
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

pub(crate) fn show(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close_transients(app);
    let source_scroll_top = app.screen.scroll_top;
    let source_scroll_left = app.screen.scroll_left;
    let source_wrap_col = app.screen.wrap_col;
    app.help_view = Some(HelpView {
        buffer: PieceTable::from_text(&help_text()),
        source_scroll_top,
        source_scroll_left,
        source_wrap_col,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.selection.clear();
    app.message = Some("Help; Esc closes.".to_string());
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_toggle(key) {
        if is_viewing(app) {
            close_with_message(app, out)?;
        } else {
            show(app, out)?;
        }
        return Ok(true);
    }
    if !is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    if key.code == KeyCode::Esc {
        close_with_message(app, out)?;
        return Ok(true);
    }
    match key.code {
        KeyCode::Left => move_cursor(app, Move::Left),
        KeyCode::Right => move_cursor(app, Move::Right),
        KeyCode::Up => move_cursor(app, Move::Up),
        KeyCode::Down => move_cursor(app, Move::Down),
        KeyCode::PageUp => move_page(app, false),
        KeyCode::PageDown => move_page(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => app.message = Some("Shortcut help is read-only; Esc closes.".to_string()),
    }
    reveal_cursor(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    app.message = Some("Shortcut help is read-only; Esc closes.".to_string());
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.help_view.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.help_view
        .as_ref()
        .map(|view| &view.buffer as &dyn Buffer)
}

fn close(app: &mut super::App) -> bool {
    let Some(view) = app.help_view.take() else {
        return false;
    };
    app.screen.scroll_top = view.source_scroll_top;
    app.screen.scroll_left = view.source_scroll_left;
    app.screen.wrap_col = view.source_wrap_col;
    true
}

fn close_with_message(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close(app);
    app.message = Some("Help closed.".to_string());
    app.reveal_cursor();
    app.render(out)
}

fn close_transients(app: &mut super::App) {
    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::llm_preview::close(app);
    super::llm_answer::close(app);
    super::recovery::close(app);
    super::external_command::cancel_all(app);
    super::repo_llm::cancel_all(app);
    super::llm_request::cancel_all(app);
    super::replace::cancel(app);
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
}

#[derive(Clone, Copy)]
enum Move {
    Left,
    Right,
    Up,
    Down,
}

fn move_cursor(app: &mut super::App, movement: Move) {
    let buffer = &mut app.help_view.as_mut().expect("help active").buffer;
    match movement {
        Move::Left => buffer.move_left(),
        Move::Right => buffer.move_right(),
        Move::Up => buffer.move_up(),
        Move::Down => buffer.move_down(),
    }
}

fn move_page(app: &mut super::App, forward: bool) {
    let movement = if forward { Move::Down } else { Move::Up };
    for _ in 0..app.screen.visible_height().max(1) {
        move_cursor(app, movement);
    }
}

fn set_line_edge(app: &mut super::App, end: bool) {
    let buffer = &mut app.help_view.as_mut().expect("help active").buffer;
    let row = buffer.cursor().row;
    let col = if end {
        buffer.line_char_count(row).unwrap_or(0)
    } else {
        0
    };
    buffer.set_cursor(Cursor { row, col });
}

fn reveal_cursor(app: &mut super::App) {
    app.reveal_cursor();
}

fn is_toggle(key: KeyEvent) -> bool {
    help_catalog::default_editor_action(key) == Some(EditorAction::Help)
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn help_text() -> String {
    let mut text = String::from(concat!(
        "Catomic help - default keyboard and command quick reference\n\n",
        "The keys below are built-in defaults. [keybindings] overrides apply only in\n",
        "normal editing mode; this view does not display effective configured keys.\n\n",
    ));
    push_editor_actions(&mut text);
    text.push_str("\nFixed and context-dependent keys\n");
    for shortcut in help_catalog::FIXED_SHORTCUTS {
        push_entry(&mut text, shortcut.keys, &[], shortcut.purpose);
    }
    text.push_str("\nPrompt commands (Ctrl+Shift+P or F2; no leading colon)\n");
    for command in help_catalog::PROMPT_COMMANDS {
        push_entry(&mut text, command.syntax, command.aliases, command.purpose);
    }
    text.push_str(concat!(
        "\nModel command context and workflow\n",
        "  Selection = highlighted text in the active file being edited.\n",
        "  Instruction block = >>> catomic ... <<< containing the cursor.\n",
        "  Plain mode = default editing; Project mode = opt-in repository tools.\n",
        "  Enter Project mode with the project command.\n",
        "  Nothing is sent until you confirm the endpoint, model, and exact context.\n",
        "  Enter confirms; Escape cancels.\n",
        "  Edit proposals open read-only; a second Enter confirms apply.\n",
        "  Model edits affect only the confirmed active file; they are not auto-saved.\n",
        "  Prefix the instruction with explain for a read-only answer.\n",
        "\nMore help\n",
        "  Configuration: $XDG_CONFIG_HOME/catomic/config.toml or\n",
        "    ~/.config/catomic/config.toml\n",
        "  User guide (configuration, terminal troubleshooting, and safety):\n",
        "    https://github.com/maelguimet/catomic/blob/master/docs/user-guide.md\n",
        "  Model setup, scopes, confirmations, and safety: user guide section\n",
        "    Model-assisted commands.\n\n",
        "Arrows, Home/End, and PageUp/PageDown navigate this read-only view.\n",
        "Escape, Ctrl+H, or F1 closes it. Ctrl+Q keeps the guarded quit path.\n",
    ));
    text
}

fn push_editor_actions(text: &mut String) {
    text.push_str("Default normal-mode shortcuts\n");
    let mut category = "";
    for action in help_catalog::EDITOR_ACTIONS {
        if action.category != category {
            category = action.category;
            text.push('\n');
            text.push_str(category);
            text.push('\n');
        }
        push_entry(text, action.default_keys, &[], action.purpose);
    }
}

fn push_entry(text: &mut String, label: &str, aliases: &[&str], purpose: &str) {
    text.push_str("  ");
    text.push_str(label);
    text.push('\n');
    if !aliases.is_empty() {
        text.push_str("    Aliases: ");
        text.push_str(&aliases.join(", "));
        text.push('\n');
    }
    text.push_str("    ");
    text.push_str(purpose);
    text.push('\n');
}

#[cfg(test)]
mod tests;
