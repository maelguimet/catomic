//! Purpose: build normalized, scoped shortcut maps from the central action registry.
//! Owns: chord parsing, overrides/unbinding, collision diagnostics, and key translation.
//! Must not: dispatch App behavior, mutate user files, spawn work, or accept printable typing.
//! Invariants: one normalized chord maps to one action per scope; global actions win.
//! Phase: issue #62 complete shortcut customization.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

use super::actions::{self, Action, InputKind, Scope};

#[derive(Clone, Debug)]
pub(crate) struct KeyBindings {
    keys: HashMap<(Scope, KeyChord), Action>,
    mouse: HashMap<(Scope, MouseGesture), Action>,
    default_keys: HashSet<(Scope, KeyChord)>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum MouseGesture {
    Left,
    LeftDrag,
    LeftUp,
    LeftDouble,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum ShortcutChord {
    Key(KeyChord),
    Mouse(MouseGesture),
}

impl Default for KeyBindings {
    fn default() -> Self {
        Builder::defaults()
            .map(Builder::finish)
            .expect("the built-in action registry must be collision-free")
    }
}

impl KeyBindings {
    pub(crate) fn translate(&self, scope: Scope, key: KeyEvent) -> Option<KeyEvent> {
        let chord = KeyChord::from_event(key);
        let action = self
            .keys
            .get(&(Scope::Global, chord))
            .or_else(|| self.keys.get(&(scope, chord)))
            .copied();
        if let Some(action) = action {
            return Some(canonical_key(action, key));
        }
        let was_default = self.default_keys.contains(&(Scope::Global, chord))
            || self.default_keys.contains(&(scope, chord));
        (!was_default).then_some(key)
    }

    pub(crate) fn mouse_action(&self, gesture: MouseGesture) -> Option<Action> {
        self.mouse.get(&(Scope::Editor, gesture)).copied()
    }
}

impl KeyChord {
    fn from_event(key: KeyEvent) -> Self {
        normalize_key(Self {
            code: key.code,
            modifiers: key.modifiers,
        })
    }
}

struct Builder {
    bindings: KeyBindings,
    origins: HashMap<(Scope, ShortcutChord), (Action, String, bool)>,
}

#[derive(Clone, Copy)]
enum InsertPolicy {
    RejectCollision,
    ReplaceDefault,
}

impl Builder {
    fn defaults() -> io::Result<Self> {
        let mut builder = Self {
            bindings: KeyBindings {
                keys: HashMap::new(),
                mouse: HashMap::new(),
                default_keys: HashSet::new(),
            },
            origins: HashMap::new(),
        };
        for entry in actions::REGISTRY {
            for raw in entry.defaults {
                let chord = parse_shortcut(raw)?;
                validate_input(entry.action, chord, raw)?;
                for scope in entry.scopes {
                    builder.insert(
                        *scope,
                        chord,
                        entry.action,
                        raw,
                        false,
                        InsertPolicy::RejectCollision,
                    )?;
                    if let ShortcutChord::Key(key) = chord {
                        builder.bindings.default_keys.insert((*scope, key));
                    }
                }
            }
        }
        Ok(builder)
    }

    fn remove_action(&mut self, action: Action) {
        self.bindings.keys.retain(|_, bound| *bound != action);
        self.bindings.mouse.retain(|_, bound| *bound != action);
        self.origins.retain(|_, (bound, _, _)| *bound != action);
    }

    fn insert(
        &mut self,
        scope: Scope,
        chord: ShortcutChord,
        action: Action,
        raw: &str,
        configured: bool,
        policy: InsertPolicy,
    ) -> io::Result<()> {
        let key = (scope, chord);
        if let Some((other, other_raw, other_configured)) = self.origins.get(&key) {
            let replaces_default =
                matches!(policy, InsertPolicy::ReplaceDefault) && !other_configured;
            if !replaces_default {
                return Err(collision(scope, *other, other_raw, action, raw, chord));
            }
        }
        self.origins
            .insert(key, (action, raw.to_string(), configured));
        match chord {
            ShortcutChord::Key(chord) => {
                self.bindings.keys.insert((scope, chord), action);
            }
            ShortcutChord::Mouse(gesture) => {
                self.bindings.mouse.insert((scope, gesture), action);
            }
        }
        Ok(())
    }

    fn finish(self) -> KeyBindings {
        self.bindings
    }
}

struct ActionOverride {
    action: Action,
    chords: Vec<(ShortcutChord, String)>,
}

struct LegacyOverride {
    chord: ShortcutChord,
    raw_chord: String,
    action: Action,
}

pub(crate) fn parse(text: &str) -> io::Result<KeyBindings> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        keybindings: toml::Table,
    }

    let table = super::decode::<ConfigFile>(text)?.keybindings;
    let (action_overrides, legacy_overrides) = decode_overrides(table)?;
    let mut builder = Builder::defaults()?;
    for configured in &action_overrides {
        builder.remove_action(configured.action);
    }
    for configured in action_overrides {
        let descriptor = actions::descriptor(configured.action);
        for (chord, raw) in configured.chords {
            for scope in descriptor.scopes {
                builder.insert(
                    *scope,
                    chord,
                    configured.action,
                    &raw,
                    true,
                    InsertPolicy::RejectCollision,
                )?;
            }
        }
    }
    for configured in legacy_overrides {
        let descriptor = actions::descriptor(configured.action);
        for scope in descriptor.scopes {
            builder.insert(
                *scope,
                configured.chord,
                configured.action,
                &configured.raw_chord,
                true,
                InsertPolicy::ReplaceDefault,
            )?;
        }
    }
    Ok(builder.finish())
}

fn decode_overrides(table: toml::Table) -> io::Result<(Vec<ActionOverride>, Vec<LegacyOverride>)> {
    let mut actions_out = Vec::new();
    let mut legacy_out = Vec::new();
    for (raw_name, value) in table {
        if let Some(action) = actions::parse_action(&raw_name) {
            let values = value.as_array().ok_or_else(|| {
                invalid(format!(
                    "keybindings.{raw_name} must be an array of chords; use [] to unbind"
                ))
            })?;
            let mut chords = Vec::new();
            for value in values {
                let raw = value.as_str().ok_or_else(|| {
                    invalid(format!("keybindings.{raw_name} chords must be strings"))
                })?;
                let chord = parse_shortcut(raw)?;
                validate_input(action, chord, raw)?;
                validate_safe_key(chord, raw)?;
                chords.push((chord, raw.to_string()));
            }
            actions_out.push(ActionOverride { action, chords });
            continue;
        }
        let raw_action = value.as_str().ok_or_else(|| {
            invalid(format!(
                "unknown keybinding action {raw_name:?}; legacy chord keys require a string action"
            ))
        })?;
        let action = actions::parse_action(raw_action)
            .ok_or_else(|| invalid(format!("unknown keybinding action {raw_action:?}")))?;
        let chord = parse_shortcut(&raw_name)?;
        validate_input(action, chord, &raw_name)?;
        validate_safe_key(chord, &raw_name)?;
        legacy_out.push(LegacyOverride {
            chord,
            raw_chord: raw_name,
            action,
        });
    }
    Ok((actions_out, legacy_out))
}

pub(crate) fn load_from(path: &Path) -> io::Result<KeyBindings> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(KeyBindings::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load() -> io::Result<KeyBindings> {
    match super::user_file::optional_path() {
        Some(path) => load_from(&path),
        None => Ok(KeyBindings::default()),
    }
}

fn canonical_key(action: Action, original: KeyEvent) -> KeyEvent {
    let descriptor = actions::descriptor(action);
    let chord = descriptor
        .defaults
        .iter()
        .find_map(
            |raw| match parse_shortcut(raw).expect("validated registry chord") {
                ShortcutChord::Key(key) => Some(key),
                ShortcutChord::Mouse(_) => None,
            },
        )
        .expect("keyboard action requires a keyboard default");
    KeyEvent {
        code: chord.code,
        modifiers: chord.modifiers,
        ..original
    }
}

fn parse_shortcut(raw: &str) -> io::Result<ShortcutChord> {
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

fn validate_input(action: Action, chord: ShortcutChord, raw: &str) -> io::Result<()> {
    let valid = matches!(
        (actions::descriptor(action).input, chord),
        (InputKind::Keyboard, ShortcutChord::Key(_)) | (InputKind::Mouse, ShortcutChord::Mouse(_))
    );
    valid.then_some(()).ok_or_else(|| {
        invalid(format!(
            "action {:?} cannot use chord {raw:?}",
            actions::descriptor(action).name
        ))
    })
}

fn validate_safe_key(chord: ShortcutChord, raw: &str) -> io::Result<()> {
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

fn collision(
    scope: Scope,
    first: Action,
    first_raw: &str,
    second: Action,
    second_raw: &str,
    chord: ShortcutChord,
) -> io::Error {
    invalid(format!(
        "keybinding collision in {}: action {:?} chord {:?} conflicts with action {:?} chord {:?} after normalization to {:?}",
        scope.name(),
        actions::descriptor(first).name,
        first_raw,
        actions::descriptor(second).name,
        second_raw,
        format_shortcut(chord)
    ))
}

fn format_shortcut(chord: ShortcutChord) -> String {
    match chord {
        ShortcutChord::Mouse(gesture) => format!("{gesture:?}"),
        ShortcutChord::Key(key) => {
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
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "keybindings/tests.rs"]
mod tests;
