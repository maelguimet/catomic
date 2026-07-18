//! Purpose: verify the built-in shortcut reference and its read-only lifecycle.
//! Owns: focused help key, navigation, and source-preservation regression tests.
//! Must not: touch disk, spawn services, access network, or depend on a real terminal.
//! Invariants: opening and closing help never changes the source buffer.
//! Phase: post-v0.1 core usability.

use crate::buffer::{Cursor, PieceTable};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::{self, Write};

use super::*;

fn app() -> crate::app::App {
    let mut app = crate::app::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("source text"));
    app
}

#[derive(Default)]
struct FrameRecorder {
    writes: Vec<Vec<u8>>,
    flushes: usize,
}

impl Write for FrameRecorder {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writes.push(buffer.to_vec());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn ctrl_h_commits_help_content_and_status_as_one_frame() {
    let mut app = app();
    app.screen.width = 120;
    app.screen.height = 50;
    let mut out = FrameRecorder::default();

    let toggle = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);
    assert!(handle_key(&mut app, &mut out, toggle).unwrap());

    assert_eq!(out.writes.len(), 1, "help redraw must be one output frame");
    assert_eq!(out.flushes, 1, "the committed frame must be flushed once");
    let frame = String::from_utf8_lossy(&out.writes[0]);
    assert!(frame.contains("Catomic help - configurable actions and command quick reference"));
    let help = display_buffer(&app).unwrap().to_string();
    assert!(help.contains("Ctrl+Z"));
    assert!(help.contains("Undo the last edit transaction."));
    assert!(help.contains("Ctrl+Y / Ctrl+Shift+Z"));
    assert!(help.contains("Redo the next edit transaction."));
    assert!(help.contains("Insert"));
    assert!(help.contains("Toggle session-wide insert/overwrite typing"));
    assert!(!help.contains("Ctrl+Z/Y"));
    assert!(frame.contains("\x1b[50;1H"));
    assert!(frame.contains("\x1b[2KHelp; Esc closes."));
    assert!(
        frame.ends_with("\x1b[0m\x1b[0 q\x1b[1;1H\x1b[?25h"),
        "frame must reset styling, select the default cursor, place it, and show it"
    );
}

#[test]
fn ctrl_h_opens_navigates_and_closes_without_editing_source() {
    let mut app = app();
    let mut out = Vec::new();
    let toggle = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);

    assert!(handle_key(&mut app, &mut out, toggle).unwrap());
    assert!(is_viewing(&app));
    let help = display_buffer(&app).unwrap().to_string();
    assert!(help.contains("save-as"));
    assert!(help.contains("Ctrl+Shift+S"));

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    )
    .unwrap();
    assert_ne!((app.screen.scroll_top, app.screen.wrap_col), (0, 0));

    assert!(handle_key(&mut app, &mut out, toggle).unwrap());
    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn rendered_help_covers_every_cataloged_shortcut_command_and_alias() {
    let text = help_text();

    for action in crate::config::actions::REGISTRY {
        assert!(text.contains(action.name), "missing action {}", action.name);
        assert!(
            text.contains(action.label),
            "missing label for {}",
            action.name
        );
        for chord in action.defaults {
            assert!(
                text.contains(&crate::config::actions::display_chord(chord)),
                "missing chord {chord} for {}",
                action.name
            );
        }
    }
    for command in crate::help_catalog::PROMPT_COMMANDS {
        assert!(
            text.contains(command.syntax),
            "missing command: {}",
            command.syntax
        );
        assert!(
            text.contains(command.purpose),
            "missing purpose: {}",
            command.syntax
        );
        for alias in command.aliases {
            assert!(text.contains(alias), "missing alias: {alias}");
        }
    }
}

#[test]
fn help_explains_context_safety_defaults_and_deeper_documentation() {
    let text = help_text();
    for required in [
        "Ctrl+R",
        "repeat only to confirm reloading the same observed revision",
        "close!",
        "Discard active-buffer edits",
        "trusted /bin/sh command; it may affect outside data",
        "Preview a newer .catnap",
        "ordinary buffers only",
        "Project mode",
        "gitmeow INSTRUCTION",
        "focused bounded repository context",
        "megameow INSTRUCTION",
        "broader bounded repository context",
        "$XDG_CONFIG_HOME/catomic/config.toml",
        "~/.config/catomic/config.toml",
        "[[llm.backends]]",
        "base_url = \"http://127.0.0.1:8080/v1\"",
        "model = \"local-model\"",
        "api_key_env = \"OPENAI_API_KEY\"",
        "OpenAI-compatible Chat Completions API",
        "api_key_env names an environment variable, never the key value itself",
        "Opening help reads no config or secret, builds no client",
        "F10 or model opens the process-local preset/model selector",
        "meow explain this",
        "bigmeow explain this file",
        "Project mode, a saved active file, and Git",
        "Inline F3",
        "Standard model commands send nothing until preset, adapter, destination",
        "autocomplete on is the only automatic-call exception",
        "read-only session confirmation with destination and bounded active-buffer",
        "context. No credential, command, client, or request starts before Enter",
        "Edit proposals open read-only; a second Enter confirms apply",
        "Model edits affect only the confirmed active file; they are not auto-saved",
        "Prefix the instruction with explain for a read-only answer",
        "does not display effective configured keys",
        "terminal troubleshooting, and safety",
        "Model-assisted commands",
        "endpoint unavailable or incompatible",
        "context over",
        "64 KiB or 2,000 lines",
    ] {
        assert!(text.contains(required), "help is missing {required:?}");
    }
}

#[test]
fn narrow_help_reaches_wrapped_model_setup_without_horizontal_scrolling() {
    let mut app = app();
    app.screen.width = 24;
    app.screen.height = 7;
    let mut out = Vec::new();
    show(&mut app, &mut out).unwrap();

    let model_row = display_buffer(&app)
        .unwrap()
        .to_string()
        .lines()
        .position(|line| line.contains("api_key_env names an environment variable"))
        .expect("model secret-indirection guidance must be present");
    app.surfaces
        .help
        .as_mut()
        .unwrap()
        .buffer
        .set_cursor(Cursor {
            row: model_row,
            col: 0,
        });
    out.clear();
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
    )
    .unwrap();

    assert!(crate::app::view::soft_wrap_active(&app));
    assert!(app.screen.scroll_top > 0);
    assert_eq!(app.screen.scroll_left, 0);
    assert!(!out.is_empty());
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn narrow_help_soft_wraps_long_safety_lines_and_keeps_navigation_bounded() {
    let mut app = app();
    app.screen.width = 24;
    app.screen.height = 7;
    let mut out = Vec::new();
    show(&mut app, &mut out).unwrap();

    assert!(crate::app::view::soft_wrap_active(&app));
    assert_eq!(app.screen.scroll_left, 0);

    let target_row = display_buffer(&app)
        .unwrap()
        .to_string()
        .lines()
        .position(|line| line.contains("trusted /bin/sh command"))
        .expect("external-command safety line must be present");
    app.surfaces
        .help
        .as_mut()
        .unwrap()
        .buffer
        .set_cursor(Cursor {
            row: target_row,
            col: 0,
        });
    out.clear();
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(
        app.screen.scroll_left, 0,
        "wrapped help never scrolls horizontally"
    );
    let frame = String::from_utf8_lossy(&out);
    assert!(frame.contains(", and output previews be"));
    assert!(frame.contains("fore any buffer edit."));
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn help_rejects_edits_and_escape_restores_source_viewport() {
    let mut app = app();
    let mut out = Vec::new();
    app.buffer = Box::new(PieceTable::from_text("a\nb\nc\nsource"));
    app.buffer.set_cursor(Cursor { row: 3, col: 3 });
    app.view.soft_wrap = true;
    app.screen.width = 4;
    app.screen.height = 2;
    app.screen.scroll_top = 3;
    app.screen.wrap_col = 2;
    show(&mut app, &mut out).unwrap();
    assert_eq!(app.screen.wrap_col, 0);
    let help_before = display_buffer(&app).unwrap().to_string();

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(display_buffer(&app).unwrap().to_string(), help_before);

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(!is_viewing(&app));
    assert_eq!(app.screen.scroll_top, 3);
    assert_eq!(app.screen.wrap_col, 2);
}
