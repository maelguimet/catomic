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
    app.surfaces.help = Some(HelpView {
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
        KeyCode::PageUp => return scroll_page(app, out, false),
        KeyCode::PageDown => return scroll_page(app, out, true),
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
    app.surfaces.help.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.surfaces
        .help
        .as_ref()
        .map(|view| &view.buffer as &dyn Buffer)
}

fn close(app: &mut super::App) -> bool {
    let Some(view) = app.surfaces.help.take() else {
        return false;
    };
    app.screen.scroll_top = view.source_scroll_top;
    app.screen.scroll_left = view.source_scroll_left;
    app.screen.wrap_col = view.source_wrap_col;
    true
}

pub(crate) fn close_for_transient(app: &mut super::App) -> bool {
    close(app)
}

fn close_with_message(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close(app);
    app.message = Some("Help closed.".to_string());
    app.reveal_cursor();
    app.render(out)
}

fn close_transients(app: &mut super::App) {
    super::autocomplete::invalidate(app);
    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::model_picker::close(app);
    super::llm_preview::close(app);
    super::llm_answer::close(app);
    super::recovery::close(app);
    super::external_command::cancel_all(app);
    super::repo_llm::cancel_all(app);
    super::llm_request::cancel_all(app);
    super::inline_clanker::cancel_all(app);
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
    let buffer = &mut app.surfaces.help.as_mut().expect("help active").buffer;
    match movement {
        Move::Left => buffer.move_left(),
        Move::Right => buffer.move_right(),
        Move::Up => buffer.move_up(),
        Move::Down => buffer.move_down(),
    }
}

fn scroll_page(app: &mut super::App, out: &mut dyn Write, forward: bool) -> io::Result<bool> {
    let direction = if forward {
        super::viewport::ScrollDirection::Down
    } else {
        super::viewport::ScrollDirection::Up
    };
    let rows = app.screen.visible_height().max(1);
    super::viewport::scroll_viewport(app, direction, rows)?;
    app.render(out)?;
    Ok(true)
}

fn set_line_edge(app: &mut super::App, end: bool) {
    let buffer = &mut app.surfaces.help.as_mut().expect("help active").buffer;
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
    let mut text = crate::config::actions::help_text();
    text.push_str("\nPrompt commands (Ctrl+Shift+P or F2; no leading colon)\n");
    for command in help_catalog::PROMPT_COMMANDS {
        push_entry(&mut text, command.syntax, command.aliases, command.purpose);
    }
    text.push_str(concat!(
        "\nUsing models - setup and examples\n",
        "  Config: $XDG_CONFIG_HOME/catomic/config.toml when XDG_CONFIG_HOME is\n",
        "    absolute; otherwise ~/.config/catomic/config.toml.\n",
        "  Minimal OpenAI-compatible preset:\n",
        "    [llm]\n",
        "    default = \"local\"\n",
        "    [[llm.backends]]\n",
        "    name = \"local\"\n",
        "    type = \"openai-compatible\"\n",
        "    base_url = \"http://127.0.0.1:8080/v1\"\n",
        "    model = \"local-model\"\n",
        "    api_key_env = \"OPENAI_API_KEY\"\n",
        "    timeout_secs = 120\n",
        "  base_url must expose an OpenAI-compatible Chat Completions API.\n",
        "  api_key_env names an environment variable, never the key value itself.\n",
        "  Loopback HTTP is allowed; authenticated remote endpoints require HTTPS.\n",
        "  Opening help reads no config or secret, builds no client, starts no\n",
        "    command, probes no endpoint, and makes no network request.\n",
        "  F10 or model opens the process-local preset/model selector without\n",
        "    contacting a backend; optional model discovery requires its own Enter.\n",
        "\nConcrete workflows (open the prompt with Ctrl+Shift+P or F2)\n",
        "  Selection: select text, run meow explain this, review the send\n",
        "    confirmation, then review the read-only answer or proposal.\n",
        "  Current file: run bigmeow explain this file; no selection is needed.\n",
        "  Repository: run project, then gitmeow INSTRUCTION or megameow\n",
        "    INSTRUCTION. These require Project mode, a saved active file, and Git.\n",
        "  Inline F3: place the cursor in a >>> catomic ... <<< instruction block;\n",
        "    confirm the bounded request, then separately confirm any apply.\n",
        "\nModel command context and workflow\n",
        "  Selection = highlighted text in the active file being edited.\n",
        "  Instruction block = >>> catomic ... <<< containing the cursor.\n",
        "  Plain mode = default editing; Project mode = opt-in repository tools.\n",
        "  Enter Project mode with the project command.\n",
        "  Standard model commands send nothing until preset, adapter, destination,\n",
        "    model, and exact context are confirmed.\n",
        "  Enter confirms; Escape cancels.\n",
        "  Edit proposals open read-only; a second Enter confirms apply.\n",
        "  Model edits affect only the confirmed active file; they are not auto-saved.\n",
        "  Prefix the instruction with explain for a read-only answer.\n",
        "  autocomplete on is the only automatic-call exception: it first opens a\n",
        "    read-only session confirmation with destination and bounded active-buffer\n",
        "    context. No credential, command, client, or request starts before Enter.\n",
        "  Confirmed suggestions are non-buffer ghost text; Tab accepts one undoable\n",
        "    edit, Escape dismisses, and typing/navigation cancels stale work.\n",
        "  Remote HTTP autocomplete additionally requires allow_remote = true.\n",
        "  Typical errors: endpoint unavailable or incompatible; missing API-key\n",
        "    environment variable; no selection/instruction block; context over\n",
        "    64 KiB or 2,000 lines; or repository commands outside Project/Git.\n",
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
