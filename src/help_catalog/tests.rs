//! Purpose: verify that public action and command metadata is complete and unambiguous.
//! Owns: catalog uniqueness, lookup, alias, binding, and help-field regression tests.
//! Must not: construct App, read configuration, touch disk, or dispatch editor actions.
//! Invariants: every catalog entry is reachable and has user-facing help.
//! Phase: post-v0.1 discoverability and help-drift prevention.

use std::collections::HashSet;

use super::*;

#[test]
fn editor_actions_have_unique_names_bindings_and_help() {
    let mut names = HashSet::new();
    let mut canonical = HashSet::new();
    for spec in EDITOR_ACTIONS {
        assert!(
            names.insert(spec.name),
            "duplicate action name: {}",
            spec.name
        );
        assert!(!spec.category.is_empty());
        assert!(!spec.default_keys.is_empty());
        assert!(!spec.purpose.is_empty());
        assert!(!spec.bindings.is_empty());
        assert_eq!(editor_action(spec.name), Some(spec.action));
        let key = canonical_key(spec.action);
        assert_eq!(default_editor_action(key), Some(spec.action));
        assert!(
            canonical.insert((key.code, key.modifiers)),
            "duplicate canonical key for {}",
            spec.name
        );
    }
}

#[test]
fn prompt_commands_and_aliases_are_unique_and_have_purposes() {
    let mut names = HashSet::new();
    for spec in PROMPT_COMMANDS {
        assert!(!spec.syntax.is_empty());
        assert!(!spec.purpose.is_empty());
        assert!(!spec.names.is_empty());
        let displayed: HashSet<_> = std::iter::once(spec.syntax)
            .chain(spec.aliases.iter().copied())
            .map(|spelling| spelling.split_ascii_whitespace().next().unwrap())
            .collect();
        for name in spec.names {
            assert!(names.insert(name), "duplicate prompt spelling: {name}");
            assert_eq!(prompt_command(name), Some(spec.command));
            assert!(
                displayed.contains(name),
                "prompt spelling is hidden from help: {name}"
            );
        }
    }
}

#[test]
fn null_and_character_forms_both_resolve_control_space() {
    for code in [KeyCode::Char(' '), KeyCode::Null] {
        assert_eq!(
            default_editor_action(KeyEvent::new(code, KeyModifiers::CONTROL)),
            Some(EditorAction::Complete)
        );
    }
}

#[test]
fn default_z_history_aliases_preserve_exact_alt_and_shift_policy() {
    assert_eq!(
        default_editor_action(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::CONTROL)),
        Some(EditorAction::Undo)
    );
    assert_eq!(
        default_editor_action(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        Some(EditorAction::Redo)
    );
    for modifiers in [
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT,
    ] {
        assert_eq!(
            default_editor_action(KeyEvent::new(KeyCode::Char('z'), modifiers)),
            None
        );
    }
}
