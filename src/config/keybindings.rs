//! Purpose: decode simple key-chord overrides into existing canonical editor actions.
//! Owns: chord parsing, action validation, and normal-mode key translation.
//! Must not: dispatch editor commands, mutate App state, spawn work, or handle prompt-local keys.
//! Invariants: one chord maps to one known action; translated keys reuse established handlers.
//! Phase: 7 keybinding configuration.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

#[derive(Clone, Debug, Default)]
pub(crate) struct KeyBindings {
    overrides: HashMap<KeyChord, Action>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Save,
    Quit,
    Reload,
    Search,
    GotoLine,
    CommandPrompt,
    Undo,
    Redo,
    Complete,
    NextBuffer,
    PreviousBuffer,
    NextPage,
    PreviousPage,
    MarkdownPreview,
    LineNumbers,
    Whitespace,
}

impl KeyBindings {
    pub(crate) fn translate(&self, key: KeyEvent) -> KeyEvent {
        self.overrides
            .get(&KeyChord::from_event(key))
            .map_or(key, |action| action.canonical_key())
    }
}

impl KeyChord {
    fn from_event(key: KeyEvent) -> Self {
        Self {
            code: normalize_code(key.code),
            modifiers: key.modifiers,
        }
    }
}

impl Action {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "save" => Self::Save,
            "quit" => Self::Quit,
            "reload" => Self::Reload,
            "search" => Self::Search,
            "goto-line" => Self::GotoLine,
            "command-prompt" => Self::CommandPrompt,
            "undo" => Self::Undo,
            "redo" => Self::Redo,
            "complete" => Self::Complete,
            "next-buffer" => Self::NextBuffer,
            "previous-buffer" => Self::PreviousBuffer,
            "next-page" => Self::NextPage,
            "previous-page" => Self::PreviousPage,
            "markdown-preview" => Self::MarkdownPreview,
            "line-numbers" => Self::LineNumbers,
            "whitespace" => Self::Whitespace,
            _ => return None,
        })
    }

    fn canonical_key(self) -> KeyEvent {
        let (code, modifiers) = match self {
            Self::Save => control_char('s'),
            Self::Quit => control_char('q'),
            Self::Reload => control_char('r'),
            Self::Search => control_char('f'),
            Self::GotoLine => control_char('g'),
            Self::CommandPrompt => (
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            Self::Undo => control_char('z'),
            Self::Redo => control_char('y'),
            Self::Complete => (KeyCode::Char(' '), KeyModifiers::CONTROL),
            Self::NextBuffer => (KeyCode::PageDown, KeyModifiers::ALT),
            Self::PreviousBuffer => (KeyCode::PageUp, KeyModifiers::ALT),
            Self::NextPage => (KeyCode::PageDown, KeyModifiers::CONTROL),
            Self::PreviousPage => (KeyCode::PageUp, KeyModifiers::CONTROL),
            Self::MarkdownPreview => (KeyCode::F(6), KeyModifiers::NONE),
            Self::LineNumbers => (KeyCode::F(7), KeyModifiers::NONE),
            Self::Whitespace => (KeyCode::F(8), KeyModifiers::NONE),
        };
        KeyEvent::new(code, modifiers)
    }
}

pub(crate) fn parse(text: &str) -> io::Result<KeyBindings> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        keybindings: HashMap<String, String>,
    }

    let mut overrides = HashMap::new();
    for (raw_chord, raw_action) in super::decode::<ConfigFile>(text)?.keybindings {
        let chord = parse_chord(&raw_chord)?;
        let action = Action::parse(&raw_action)
            .ok_or_else(|| invalid(format!("unknown keybinding action {raw_action:?}")))?;
        overrides.insert(chord, action);
    }
    Ok(KeyBindings { overrides })
}

pub(crate) fn load_from(path: &Path) -> io::Result<KeyBindings> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(KeyBindings::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<KeyBindings> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    match super::big_files::config_path(xdg.as_deref(), home.as_deref()) {
        Some(path) => load_from(&path),
        None => Ok(KeyBindings::default()),
    }
}

fn parse_chord(raw: &str) -> io::Result<KeyChord> {
    let mut modifiers = KeyModifiers::NONE;
    let mut code = None;
    for token in raw.split('+').map(str::trim) {
        let normalized = token.to_ascii_lowercase();
        let modifier = match normalized.as_str() {
            "ctrl" | "control" => Some(KeyModifiers::CONTROL),
            "alt" => Some(KeyModifiers::ALT),
            "shift" => Some(KeyModifiers::SHIFT),
            _ => None,
        };
        if let Some(modifier) = modifier {
            if modifiers.contains(modifier) {
                return Err(invalid(format!("duplicate modifier in {raw:?}")));
            }
            modifiers.insert(modifier);
        } else if code.replace(parse_code(&normalized)?).is_some() {
            return Err(invalid(format!("multiple keys in chord {raw:?}")));
        }
    }
    Ok(KeyChord {
        code: code.ok_or_else(|| invalid(format!("missing key in chord {raw:?}")))?,
        modifiers,
    })
}

fn parse_code(name: &str) -> io::Result<KeyCode> {
    let code = match name {
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        _ if name.chars().count() == 1 => KeyCode::Char(name.chars().next().unwrap()),
        _ if name.starts_with('f') => parse_function_key(name)?,
        _ => return Err(invalid(format!("unknown key {name:?}"))),
    };
    Ok(code)
}

fn parse_function_key(name: &str) -> io::Result<KeyCode> {
    let number = name[1..]
        .parse::<u8>()
        .map_err(|_| invalid(format!("unknown key {name:?}")))?;
    if !(1..=12).contains(&number) {
        return Err(invalid("function key must be f1 through f12"));
    }
    Ok(KeyCode::F(number))
}

fn normalize_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(ch) if ch.is_ascii_uppercase() => KeyCode::Char(ch.to_ascii_lowercase()),
        other => other,
    }
}

fn control_char(ch: char) -> (KeyCode, KeyModifiers) {
    (KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_configuration_keeps_keys_unchanged() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);

        assert_eq!(KeyBindings::default().translate(key), key);
    }

    #[test]
    fn configured_chords_translate_to_canonical_actions() {
        let bindings =
            parse("[keybindings]\n\"ctrl+w\" = \"save\"\n\"alt+shift+p\" = \"command-prompt\"\n")
                .unwrap();

        assert_eq!(
            bindings.translate(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            bindings.translate(KeyEvent::new(
                KeyCode::Char('P'),
                KeyModifiers::ALT | KeyModifiers::SHIFT
            )),
            KeyEvent::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )
        );
    }

    #[test]
    fn rejects_unknown_actions_and_ambiguous_chords() {
        for text in [
            "[keybindings]\n\"ctrl+x\" = \"explode\"\n",
            "[keybindings]\n\"ctrl+ctrl+x\" = \"save\"\n",
            "[keybindings]\n\"ctrl+x+y\" = \"save\"\n",
            "[keybindings]\n\"ctrl+f13\" = \"save\"\n",
        ] {
            assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
