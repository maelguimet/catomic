//! Purpose: present the built-in task reference as rendered, read-only Markdown.
//! Owns: curated help content, local search, navigation, and source viewport restoration.
//! Must not: mutate source/history, read configuration, spawn work, or access network.
//! Invariants: Ctrl+H/F1 toggle the view; Escape closes it; all content is read-only.

use std::fmt::Write as _;
use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::config::actions::Action;
use crate::config::keybindings::KeyBindings;
use crate::editor::search::{self, SearchDirection, SearchMatch};
use crate::editor::syntax::{HyperlinkSpan, StyledSpan};

pub(crate) struct HelpView {
    buffer: PieceTable,
    spans: Vec<Vec<StyledSpan>>,
    links: Vec<Vec<HyperlinkSpan>>,
    search: HelpSearch,
    source_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

#[derive(Default)]
struct HelpSearch {
    prompt: Option<String>,
    origin: Option<Cursor>,
    active_match: Option<SearchMatch>,
}

pub(crate) fn show(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close_transients(app);
    let source_scroll_top = app.screen.scroll_top;
    let source_scroll_left = app.screen.scroll_left;
    let source_wrap_col = app.screen.wrap_col;
    let markdown = help_markdown(&app.keybindings);
    let rendered = crate::editor::markdown_preview::render_with_width(
        &markdown,
        super::view::content_width(app),
    )
    .map_err(|error| io::Error::other(error.to_string()))?;
    app.surfaces.help = Some(HelpView {
        buffer: PieceTable::from_owned_text(rendered.text),
        spans: rendered.spans,
        links: rendered.links,
        search: HelpSearch::default(),
        source_scroll_top,
        source_scroll_left,
        source_wrap_col,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.selection.clear();
    app.message_info("Help; Esc closes.");
    app.render(out)
}

pub(crate) fn presentation(
    app: &super::App,
) -> Option<crate::terminal::render::DocumentPresentation<'_>> {
    app.surfaces
        .help
        .as_ref()
        .map(|view| crate::terminal::render::DocumentPresentation {
            spans: &view.spans,
            links: &view.links,
        })
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if !is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    if is_searching(app) {
        return handle_search_key(app, out, key);
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
        _ => app.message_info("Shortcut help is read-only; Esc closes."),
    }
    reveal_cursor(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    app.message_info("Shortcut help is read-only; Esc closes.");
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

pub(crate) fn active_search_match(app: &super::App) -> Option<SearchMatch> {
    app.surfaces
        .help
        .as_ref()
        .and_then(|view| view.search.active_match)
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
    app.message = None;
    app.reveal_cursor();
    app.render(out)
}

fn close_transients(app: &mut super::App) {
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

pub(crate) fn toggle(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if is_viewing(app) {
        close_with_message(app, out)
    } else {
        show(app, out)
    }
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    match action {
        Action::Search => return open_search(app, out).map(|()| true),
        Action::HelpClose => return close_with_message(app, out).map(|()| true),
        Action::MoveLeft => move_cursor(app, Move::Left),
        Action::MoveRight => move_cursor(app, Move::Right),
        Action::MoveUp => move_cursor(app, Move::Up),
        Action::MoveDown => move_cursor(app, Move::Down),
        Action::ViewportUp => return scroll_page(app, out, false),
        Action::ViewportDown => return scroll_page(app, out, true),
        Action::LineStart => set_line_edge(app, false),
        Action::LineEnd => set_line_edge(app, true),
        _ => return Ok(false),
    }
    reveal_cursor(app);
    app.render(out)?;
    Ok(true)
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn help_markdown(bindings: &KeyBindings) -> String {
    let mut markdown = String::from("# Catomic help\n\n");
    markdown.push_str("A compact reference for the workflows worth remembering. ");
    let search_chords = display_chords(bindings, Action::Search);
    if !search_chords.is_empty() {
        let _ = write!(
            markdown,
            "Search this page with {}. ",
            search_chords.join(" / ")
        );
    }
    markdown.push_str("Press `Esc` to close it.\n\n## Files and buffers\n\n");
    push_file_actions(&mut markdown, bindings);
    markdown.push_str("\n## Edit and navigate\n\n");
    push_edit_actions(&mut markdown, bindings);
    markdown.push_str("\n## Commands and views\n\n");
    push_command_actions(&mut markdown, bindings);
    push_external_change_help(&mut markdown);
    push_model_help(&mut markdown, bindings);
    markdown.push_str(
        "Configuration, model setup, Project commands, mobile controls, and troubleshooting live in the [user guide](https://github.com/maelguimet/catomic/blob/master/docs/user-guide.md).\n",
    );
    markdown
}

fn push_file_actions(markdown: &mut String, bindings: &KeyBindings) {
    push_action(markdown, bindings, Action::Save, "Save");
    push_action(markdown, bindings, Action::SaveAs, "Save As");
    push_action(markdown, bindings, Action::Open, "Open");
    push_action(markdown, bindings, Action::New, "New");
    push_action(markdown, bindings, Action::Close, "Close");
    push_action(
        markdown,
        bindings,
        Action::PreviousBuffer,
        "Previous buffer",
    );
    push_action(markdown, bindings, Action::NextBuffer, "Next buffer");
    push_action(markdown, bindings, Action::Quit, "Quit");
    push_action(markdown, bindings, Action::Interrupt, "Interrupt");
}

fn push_edit_actions(markdown: &mut String, bindings: &KeyBindings) {
    push_action(markdown, bindings, Action::Undo, "Undo");
    push_action(markdown, bindings, Action::Redo, "Redo");
    push_action(markdown, bindings, Action::CutLine, "Cut line");
    push_action(markdown, bindings, Action::Search, "Find");
    push_action(markdown, bindings, Action::Replace, "Replace");
    push_action(markdown, bindings, Action::GotoLine, "Go to line");
}

fn push_command_actions(markdown: &mut String, bindings: &KeyBindings) {
    push_action(markdown, bindings, Action::CommandPrompt, "Command palette");
    push_action(
        markdown,
        bindings,
        Action::ToggleExternalDiff,
        "External change marks",
    );
    push_action(
        markdown,
        bindings,
        Action::MarkdownPreview,
        "Markdown preview",
    );
}

fn push_external_change_help(markdown: &mut String) {
    markdown.push_str(concat!(
        "\n## External changes and recovery\n\n",
        "- Clean buffers reload automatically after an external change unless auto-reload is disabled.\n",
        "- Dirty buffers are never replaced automatically. Use the reload action twice to accept the same observed disk revision.\n",
        "- Saves are atomic. If the file changed on disk, the second save succeeds only while that observed state is unchanged.\n",
        "- When crash recovery is enabled, run `recover` to preview a newer `.catnap`; applying it is explicit and undoable.\n",
    ));
}

fn push_model_help(markdown: &mut String, bindings: &KeyBindings) {
    markdown.push_str("\n## Models\n\n");
    push_action(markdown, bindings, Action::SelectModel, "Select model");
    markdown.push_str(
        "- Model requests show the destination and bounded context before sending. Proposals are read-only until separately applied and are never auto-saved.\n\n",
    );
}

fn push_action(markdown: &mut String, bindings: &KeyBindings, action: Action, label: &str) {
    let purpose = crate::config::actions::descriptor(action).help;
    let chords = display_chords(bindings, action);
    if chords.is_empty() {
        let _ = writeln!(markdown, "- **{label}** — {purpose}");
    } else {
        let _ = writeln!(
            markdown,
            "- **{label}** ({}) — {purpose}",
            chords.join(" / ")
        );
    }
}

fn display_chords(bindings: &KeyBindings, action: Action) -> Vec<String> {
    bindings
        .keyboard_chords(action)
        .iter()
        .map(|chord| format!("`{}`", crate::config::actions::display_chord(chord)))
        .collect()
}

pub(crate) fn is_searching(app: &super::App) -> bool {
    app.surfaces
        .help
        .as_ref()
        .is_some_and(|view| view.search.prompt.is_some())
}

fn open_search(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let view = app.surfaces.help.as_mut().expect("help active");
    view.search.prompt = Some(String::new());
    view.search.origin = Some(view.buffer.cursor());
    view.search.active_match = None;
    app.message_info("Find help: ");
    app.render(out)
}

fn handle_search_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => cancel_search(app),
        KeyCode::Enter | KeyCode::Down => find_help_match(app, SearchDirection::Forward, false),
        KeyCode::Up => find_help_match(app, SearchDirection::Backward, false),
        KeyCode::Backspace => {
            app.surfaces
                .help
                .as_mut()
                .expect("help active")
                .search
                .prompt
                .as_mut()
                .expect("help search active")
                .pop();
            find_help_match(app, SearchDirection::Forward, true);
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            let ch = if key.modifiers.contains(KeyModifiers::SHIFT) && ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            if !ch.is_control() {
                app.surfaces
                    .help
                    .as_mut()
                    .expect("help active")
                    .search
                    .prompt
                    .as_mut()
                    .expect("help search active")
                    .push(ch);
            }
            find_help_match(app, SearchDirection::Forward, true);
        }
        _ => {}
    }
    app.render(out)?;
    Ok(true)
}

fn cancel_search(app: &mut super::App) {
    let view = app.surfaces.help.as_mut().expect("help active");
    if let Some(origin) = view.search.origin.take() {
        view.buffer.set_cursor(origin);
    }
    view.search.prompt = None;
    view.search.active_match = None;
    app.message_info("Help; Esc closes.");
    app.reveal_cursor();
}

fn find_help_match(app: &mut super::App, direction: SearchDirection, include_origin: bool) {
    let (query, origin) = {
        let view = app.surfaces.help.as_ref().expect("help active");
        (
            view.search.prompt.clone().unwrap_or_default(),
            if include_origin {
                view.search.origin.unwrap_or_else(|| view.buffer.cursor())
            } else {
                view.search
                    .active_match
                    .map(|found| found.start)
                    .unwrap_or_else(|| view.buffer.cursor())
            },
        )
    };
    if query.is_empty() {
        let view = app.surfaces.help.as_mut().expect("help active");
        view.search.active_match = None;
        if let Some(origin) = view.search.origin {
            view.buffer.set_cursor(origin);
        }
        app.message_info("Find help: ");
        app.reveal_cursor();
        return;
    }
    let found = {
        let view = app.surfaces.help.as_ref().expect("help active");
        search::find_match(&view.buffer, &query, origin, direction, include_origin)
    };
    let view = app.surfaces.help.as_mut().expect("help active");
    view.search.active_match = found;
    if let Some(found) = found {
        view.buffer.set_cursor(found.start);
        app.message_info(format!(
            "Found '{query}'. Enter/Down next, Up previous, Esc closes search."
        ));
        app.reveal_cursor();
    } else {
        app.message_info(format!("No matches for '{query}'. Esc closes search."));
    }
}

#[cfg(test)]
mod tests;
