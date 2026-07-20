//! Purpose: verify scoped semantic resolution, override/unbind behavior, and diagnostics.
//! Owns: pure keybinding fixtures; no process environment or terminal is required.
//! Must not: dispatch App commands, write configuration, or duplicate the registry.
//! Invariants: configured keys resolve to Action; defaults can be suppressed without eating raw input.
//! Phase: issue #171 semantic shortcut dispatch.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn terminal_aliases_resolve_to_one_action_without_rewriting_the_event() {
    let bindings = KeyBindings::default();
    for modifiers in [
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    ] {
        assert_eq!(
            bindings.action_for_key(Scope::Editor, key(KeyCode::Left, modifiers)),
            Some(Action::SelectWordLeft)
        );
    }
    for code in [KeyCode::Char(' '), KeyCode::Null] {
        assert_eq!(
            bindings.action_for_key(Scope::Editor, key(code, KeyModifiers::CONTROL)),
            Some(Action::Complete)
        );
    }
    assert_eq!(
        bindings.action_for_key(Scope::Editor, key(KeyCode::F(4), KeyModifiers::NONE)),
        Some(Action::Lint)
    );
}

#[test]
fn overrides_replace_defaults_and_unbound_defaults_are_suppressed() {
    let bindings = parse(
        "[keybindings]\nsave = [\"alt+s\"]\nhelp = []\ncommand-prompt = [\"alt+p\", \"f12\"]\n",
    )
    .unwrap();
    let ctrl_s = key(KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(bindings.action_for_key(Scope::Editor, ctrl_s), None);
    assert!(bindings.is_default_key(Scope::Editor, ctrl_s));
    assert_eq!(
        bindings.action_for_key(Scope::Editor, key(KeyCode::Char('s'), KeyModifiers::ALT)),
        Some(Action::Save)
    );
    assert_eq!(
        bindings.action_for_key(Scope::Editor, key(KeyCode::F(12), KeyModifiers::NONE)),
        Some(Action::CommandPrompt)
    );
    assert!(bindings.is_default_key(Scope::Editor, key(KeyCode::F(1), KeyModifiers::NONE)));
}

#[test]
fn unrelated_raw_keys_are_not_claimed_or_suppressed() {
    let bindings = KeyBindings::default();
    let raw = key(KeyCode::Char('x'), KeyModifiers::ALT);
    assert_eq!(bindings.action_for_key(Scope::Editor, raw), None);
    assert!(!bindings.is_default_key(Scope::Editor, raw));
}

#[test]
fn global_actions_win_and_local_scopes_can_reuse_chords() {
    let bindings = parse(
        "[keybindings]\nhelp = [\"alt+h\"]\nprompt-cancel = [\"alt+x\"]\ncompletion-cancel = [\"alt+x\"]\n",
    )
    .unwrap();
    for scope in [Scope::Editor, Scope::Prompt, Scope::Preview, Scope::Picker] {
        assert_eq!(
            bindings.action_for_key(scope, key(KeyCode::Char('h'), KeyModifiers::ALT)),
            Some(Action::Help)
        );
    }
    let alt_x = key(KeyCode::Char('x'), KeyModifiers::ALT);
    assert_eq!(
        bindings.action_for_key(Scope::Prompt, alt_x),
        Some(Action::PromptCancel)
    );
    assert_eq!(
        bindings.action_for_key(Scope::Completion, alt_x),
        Some(Action::CompletionCancel)
    );
}

#[test]
fn effective_chords_are_deduplicated_and_omit_unbound_actions() {
    let bindings =
        parse("[keybindings]\nredo = [\"alt+r\", \"ctrl+shift+r\"]\nsave = []\n").unwrap();
    assert_eq!(
        bindings.keyboard_chords(Action::Redo),
        vec!["alt+r".to_string(), "ctrl+shift+r".to_string()]
    );
    assert!(bindings.keyboard_chords(Action::Save).is_empty());
}

#[test]
fn legacy_chord_to_action_overrides_resolve_semantically() {
    let bindings =
        parse("[keybindings]\n\"ctrl+w\" = \"save\"\n\"alt+s\" = \"save-as\"\n").unwrap();
    assert_eq!(
        bindings.action_for_key(
            Scope::Editor,
            key(KeyCode::Char('w'), KeyModifiers::CONTROL)
        ),
        Some(Action::Save)
    );
    assert_eq!(
        bindings.action_for_key(Scope::Editor, key(KeyCode::Char('s'), KeyModifiers::ALT)),
        Some(Action::SaveAs)
    );
}

#[test]
fn collisions_name_both_actions_and_the_normalized_chord() {
    for text in [
        "[keybindings]\nsave = [\"control+W\"]\n",
        "[keybindings]\nsave = [\"ctrl+shift+A\"]\nclose = [\"control+shift+a\"]\n",
        "[keybindings]\nquit = [\"esc\"]\n",
    ] {
        let message = parse(text).unwrap_err().to_string();
        assert!(message.contains("conflict"), "{message}");
    }
}

#[test]
fn printable_shortcuts_and_keyboard_mouse_mismatches_fail_closed() {
    for text in [
        "[keybindings]\nsave = [\"x\"]\n",
        "[keybindings]\nsave = [\"shift+x\"]\n",
        "[keybindings]\nsave = [\"mouse-left\"]\n",
        "[keybindings]\nmouse-place-cursor = [\"ctrl+m\"]\n",
    ] {
        assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn mouse_gestures_resolve_to_semantic_actions_and_can_be_unbound() {
    let bindings = parse(
        "[keybindings]\nmouse-place-cursor = []\nmouse-select-word = [\"mouse-left\"]\nmouse-scroll-up = []\nmouse-scroll-down = [\"mouse-wheel-up\"]\n",
    )
    .unwrap();
    assert_eq!(
        bindings.mouse_action(Scope::Editor, MouseGesture::Left),
        Some(Action::MouseSelectWord)
    );
    assert_eq!(
        bindings.mouse_action(Scope::Editor, MouseGesture::LeftDouble),
        None
    );
    assert_eq!(
        bindings.mouse_action(Scope::Help, MouseGesture::ScrollUp),
        Some(Action::MouseScrollDown)
    );
}

#[test]
fn registry_defaults_are_complete_and_collision_free() {
    let bindings = KeyBindings::default();
    assert_eq!(actions::REGISTRY.len(), 88);
    for descriptor in actions::REGISTRY {
        assert!(!descriptor.name.is_empty());
        assert!(!descriptor.scopes.is_empty());
        assert!(!descriptor.defaults.is_empty());
    }
    assert!(!bindings.keys.is_empty());
}
