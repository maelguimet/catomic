//! Purpose: build normalized, scoped shortcut maps from the central action registry.
//! Owns: overrides/unbinding, scoped collision diagnostics, and key translation.
//! Must not: dispatch App behavior, mutate user files, spawn work, or accept printable typing.
//! Invariants: one normalized chord maps to one action per scope; global actions win.
//! Phase: issue #62 complete shortcut customization.

use std::collections::{HashMap, HashSet};
use std::io;

use crossterm::event::KeyEvent;
use serde::Deserialize;

use super::actions::{self, Action, InputKind, Scope};
use chord::{format_shortcut, parse_shortcut, validate_safe_key, KeyChord, ShortcutChord};

mod chord;
pub(crate) use chord::MouseGesture;

#[derive(Clone, Debug)]
pub(crate) struct KeyBindings {
    keys: HashMap<(Scope, KeyChord), Action>,
    mouse: HashMap<(Scope, MouseGesture), Action>,
    default_keys: HashSet<(Scope, KeyChord)>,
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

    pub(crate) fn mouse_action(&self, scope: Scope, gesture: MouseGesture) -> Option<Action> {
        self.mouse
            .get(&(Scope::Global, gesture))
            .or_else(|| self.mouse.get(&(scope, gesture)))
            .copied()
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
        let cross_scope_conflicts = self.cross_scope_conflicts(scope, chord);
        if let Some((_, report_scope, other, other_raw, _)) =
            cross_scope_conflicts
                .iter()
                .find(|(_, _, _, _, configured)| {
                    !matches!(policy, InsertPolicy::ReplaceDefault) || *configured
                })
        {
            return Err(collision(
                *report_scope,
                *other,
                other_raw,
                action,
                raw,
                chord,
            ));
        }
        for (stored_scope, _, _, _, _) in cross_scope_conflicts {
            self.remove_chord(stored_scope, chord);
        }
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

    fn cross_scope_conflicts(
        &self,
        scope: Scope,
        chord: ShortcutChord,
    ) -> Vec<(Scope, Scope, Action, String, bool)> {
        const LOCAL_SCOPES: &[Scope] = &[
            Scope::Editor,
            Scope::Prompt,
            Scope::Search,
            Scope::Completion,
            Scope::Preview,
            Scope::Picker,
            Scope::Help,
        ];
        if scope == Scope::Global {
            return LOCAL_SCOPES
                .iter()
                .filter_map(|local| {
                    self.origins
                        .get(&(*local, chord))
                        .map(|(action, raw, configured)| {
                            (*local, *local, *action, raw.clone(), *configured)
                        })
                })
                .collect();
        }
        self.origins
            .get(&(Scope::Global, chord))
            .map(|(action, raw, configured)| {
                vec![(Scope::Global, scope, *action, raw.clone(), *configured)]
            })
            .unwrap_or_default()
    }

    fn remove_chord(&mut self, scope: Scope, chord: ShortcutChord) {
        self.origins.remove(&(scope, chord));
        match chord {
            ShortcutChord::Key(key) => {
                self.bindings.keys.remove(&(scope, key));
            }
            ShortcutChord::Mouse(gesture) => {
                self.bindings.mouse.remove(&(scope, gesture));
            }
        }
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

fn validate_input(action: Action, chord: ShortcutChord, raw: &str) -> io::Result<()> {
    let valid = matches!(
        (actions::descriptor(action).input, chord),
        (InputKind::Keyboard, ShortcutChord::Key(_))
            | (
                InputKind::MouseButton,
                ShortcutChord::Mouse(
                    MouseGesture::Left
                        | MouseGesture::LeftDrag
                        | MouseGesture::LeftUp
                        | MouseGesture::LeftDouble,
                ),
            )
            | (
                InputKind::MouseWheel,
                ShortcutChord::Mouse(MouseGesture::ScrollUp | MouseGesture::ScrollDown),
            )
    );
    valid.then_some(()).ok_or_else(|| {
        invalid(format!(
            "action {:?} cannot use chord {raw:?}",
            actions::descriptor(action).name
        ))
    })
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

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "keybindings/tests.rs"]
mod tests;
