//! Purpose: normalize and format keyboard and mouse shortcut chords.
//! Owns: typed chord forms, parser aliases, printable-key safety, and display diagnostics.
//! Must not: inspect action scopes, build binding maps, read configuration, or dispatch behavior.
//! Invariants: equivalent terminal encodings normalize identically; unsafe printable remaps fail.
//! Phase: issue #62 complete shortcut customization.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) struct KeyChord {
    pub(super) code: KeyCode,
    pub(super) modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum MouseGesture {
    Left,
    LeftDrag,
    LeftUp,
    LeftDouble,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum ShortcutChord {
    Key(KeyChord),
    Mouse(MouseGesture),
}

impl KeyChord {
    pub(super) fn from_event(key: KeyEvent) -> Self {
        normalize_key(Self {
            code: key.code,
            modifiers: key.modifiers,
        })
    }
}

pub(super) fn parse_shortcut(raw: &str) -> io::Result<ShortcutChord> {
    let normalized = raw.trim().to_ascii_lowercase();
    if let Some(mouse) = parse_mouse(&normalized) {
        return Ok(ShortcutChord::Mouse(mouse));
    }
    parse_key(&normalized).map(ShortcutChord::Key)
}

fn parse_mouse(name: &str) -> Option<MouseGesture> {
    Some(match name {
        "mouse-left" => MouseGesture::Left,
        "mouse-left-drag" => MouseGesture::LeftDrag,
        "mouse-left-up" => MouseGesture::LeftUp,
        "mouse-left-double" => MouseGesture::LeftDouble,
        "mouse-wheel-up" => MouseGesture::ScrollUp,
        "mouse-wheel-down" => MouseGesture::ScrollDown,
        _ => return None,
    })
}

fn parse_key(raw: &str) -> io::Result<KeyChord> {
    let mut modifiers = KeyModifiers::NONE;
    let mut code = None;
    for token in raw.split('+').map(str::trim) {
        let modifier = match token {
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
        } else if code.replace(parse_code(token)?).is_some() {
            return Err(invalid(format!("multiple keys in chord {raw:?}")));
        }
    }
    let code = code.ok_or_else(|| invalid(format!("missing key in chord {raw:?}")))?;
    Ok(normalize_key(KeyChord { code, modifiers }))
}

fn parse_code(name: &str) -> io::Result<KeyCode> {
    Ok(match name {
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
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
    })
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

fn normalize_key(mut chord: KeyChord) -> KeyChord {
    chord.modifiers &= KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
    match chord.code {
        KeyCode::Char(ch) if ch.is_ascii_uppercase() => {
            chord.code = KeyCode::Char(ch.to_ascii_lowercase());
        }
        KeyCode::BackTab => {
            chord.code = KeyCode::Tab;
            chord.modifiers.insert(KeyModifiers::SHIFT);
        }
        KeyCode::Null if chord.modifiers.contains(KeyModifiers::CONTROL) => {
            chord.code = KeyCode::Char(' ');
        }
        _ => {}
    }
    chord
}

pub(super) fn validate_safe_key(chord: ShortcutChord, raw: &str) -> io::Result<()> {
    let ShortcutChord::Key(key) = chord else {
        return Ok(());
    };
    let printable = matches!(key.code, KeyCode::Char(_))
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    if printable {
        return Err(invalid(format!(
            "refusing printable keybinding {raw:?}; use ctrl or alt so normal typing stays safe"
        )));
    }
    Ok(())
}

pub(super) fn format_shortcut(chord: ShortcutChord) -> String {
    match chord {
        ShortcutChord::Mouse(gesture) => format!("{gesture:?}"),
        ShortcutChord::Key(key) => format_key(key),
    }
}

fn format_key(key: KeyChord) -> String {
    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift".to_string());
    }
    parts.push(match key.code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::F(number) => format!("f{number}"),
        other => format!("{other:?}"),
    });
    parts.join("+")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
