//! Purpose: implement session-wide insert/overwrite behavior for direct typing.
//! Owns: mode toggling, grapheme replacement selection, and overwrite-cursor eligibility.
//! Must not: affect paste, indentation, completion, command/model apply, prompts, or read-only views.
//! Invariants: overwrite replaces one same-line grapheme; line ends always insert.
//! Phase: post-v0.1 explicit overwrite mode.

use std::io::{self, Write};

use crate::editor::text_layout;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TypingMode {
    #[default]
    Insert,
    Overwrite,
}

impl TypingMode {
    pub(crate) fn is_overwrite(self) -> bool {
        self == Self::Overwrite
    }
}

pub(crate) fn toggle(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    app.typing_mode = match app.typing_mode {
        TypingMode::Insert => TypingMode::Overwrite,
        TypingMode::Overwrite => TypingMode::Insert,
    };
    app.message = None;
    app.render(out)
}

pub(crate) fn type_char(app: &mut super::App, ch: char) -> io::Result<()> {
    if super::selection::replace_active(app, &ch.to_string())? {
        return Ok(());
    }
    if !app.typing_mode.is_overwrite() {
        app.buffer.insert_char(ch);
        return Ok(());
    }
    super::navigation::snap_current_grapheme(app)?;
    if continues_previous_grapheme(app, ch)? {
        app.buffer.insert_char(ch);
        return Ok(());
    }
    let start = app.buffer.cursor();
    let line_len = app.buffer.line_char_count(start.row).unwrap_or(0);
    if start.col >= line_len {
        app.buffer.insert_char(ch);
        return Ok(());
    }
    let end = super::navigation::next_grapheme_cursor(&*app.buffer)?;
    debug_assert_eq!(start.row, end.row, "non-EOL grapheme stays on its line");
    app.buffer.replace_range(start, end, &ch.to_string())?;
    Ok(())
}

fn continues_previous_grapheme(app: &super::App, ch: char) -> io::Result<bool> {
    let end = app.buffer.cursor();
    if end.col == 0 {
        return Ok(false);
    }
    let start = super::navigation::previous_grapheme_cursor(&*app.buffer)?;
    if start.row != end.row {
        return Ok(false);
    }
    let previous = app.buffer.text_range(start, end)?;
    Ok(text_layout::continues_grapheme(&previous, ch))
}

pub(crate) fn uses_overwrite_cursor(app: &super::App) -> bool {
    app.typing_mode.is_overwrite() && !alternate_input_surface(app)
}

fn alternate_input_surface(app: &super::App) -> bool {
    app.buffer.is_read_only()
        || super::help::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
        || super::repo_llm::blocks_editing_input(app)
        || app.pending_llm_request.is_some()
        || super::replace::is_active(app)
        || super::search::is_active(app)
        || super::command_prompt::is_active(app)
        || super::llm_preview::is_viewing(app)
        || super::llm_answer::is_viewing(app)
        || super::completion::is_active(app)
        || super::project_files::is_viewing(app)
        || super::lint::is_viewing(app)
        || super::view::is_preview(app)
}
