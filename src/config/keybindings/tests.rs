//! Purpose: verify normalized scoped defaults, override/unbind behavior, and diagnostics.
//! Owns: pure keybinding fixtures; no process environment or terminal is required.
//! Must not: dispatch App commands, write configuration, or duplicate the registry.
//! Invariants: tests compare semantic translation and actionable error content.
//! Phase: issue #62 complete shortcut customization.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn action_overrides_replace_all_defaults_and_empty_arrays_unbind() {
    let bindings = parse(
        "[keybindings]\nsave = [\"alt+s\"]\nhelp = []\ncommand-prompt = [\"alt+p\", \"f3\"]\n",
    )
    .unwrap();

    assert!(bindings
        .translate(
            Scope::Editor,
            key(KeyCode::Char('s'), KeyModifiers::CONTROL)
        )
        .is_none());
    assert_eq!(
        bindings.translate(Scope::Editor, key(KeyCode::Char('s'), KeyModifiers::ALT)),
        Some(key(KeyCode::Char('s'), KeyModifiers::CONTROL))
    );
    assert!(bindings
        .translate(Scope::Editor, key(KeyCode::F(1), KeyModifiers::NONE))
        .is_none());
    assert_eq!(
        bindings.translate(Scope::Editor, key(KeyCode::F(3), KeyModifiers::NONE)),
        Some(key(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ))
    );
}

#[test]
fn legacy_chord_to_action_overrides_remain_compatible() {
    let bindings = parse(
        "[keybindings]\n\"ctrl+w\" = \"save\"\n\"alt+s\" = \"save-as\"\n\"alt+shift+p\" = \"command-prompt\"\n",
    )
    .unwrap();

    assert_eq!(
        bindings.translate(
            Scope::Editor,
            key(KeyCode::Char('w'), KeyModifiers::CONTROL)
        ),
        Some(key(KeyCode::Char('s'), KeyModifiers::CONTROL))
    );
    assert_eq!(
        bindings.translate(Scope::Editor, key(KeyCode::Char('s'), KeyModifiers::ALT)),
        Some(key(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ))
    );
}

#[test]
fn legacy_overrides_really_replace_cross_scope_defaults() {
    let local_replaces_global = parse("[keybindings]\n\"ctrl+h\" = \"save\"\n").unwrap();
    assert_eq!(
        local_replaces_global.translate(
            Scope::Editor,
            key(KeyCode::Char('h'), KeyModifiers::CONTROL)
        ),
        Some(key(KeyCode::Char('s'), KeyModifiers::CONTROL))
    );
    assert!(local_replaces_global
        .translate(
            Scope::Prompt,
            key(KeyCode::Char('h'), KeyModifiers::CONTROL)
        )
        .is_none());

    let global_replaces_locals = parse("[keybindings]\nenter = \"help\"\n").unwrap();
    for scope in [
        Scope::Editor,
        Scope::Prompt,
        Scope::Search,
        Scope::Completion,
    ] {
        assert_eq!(
            global_replaces_locals.translate(scope, key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(key(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            "global override must win in {}",
            scope.name()
        );
    }
}

#[test]
fn local_scope_is_predictable_and_global_actions_win_everywhere() {
    let bindings =
        parse("[keybindings]\nprompt-cancel = [\"alt+x\"]\nquit = [\"alt+q\"]\n").unwrap();
    let alt_x = key(KeyCode::Char('x'), KeyModifiers::ALT);
    assert_eq!(bindings.translate(Scope::Editor, alt_x), Some(alt_x));
    assert_eq!(
        bindings.translate(Scope::Prompt, alt_x),
        Some(key(KeyCode::Esc, KeyModifiers::NONE))
    );
    assert_eq!(
        bindings.translate(Scope::Preview, key(KeyCode::Char('q'), KeyModifiers::ALT)),
        Some(key(KeyCode::Char('q'), KeyModifiers::CONTROL))
    );
}

#[test]
fn separate_local_scopes_may_intentionally_reuse_a_chord() {
    let bindings =
        parse("[keybindings]\nprompt-cancel = [\"alt+x\"]\ncompletion-cancel = [\"alt+x\"]\n")
            .unwrap();
    let chord = key(KeyCode::Char('x'), KeyModifiers::ALT);
    assert_eq!(
        bindings.translate(Scope::Prompt, chord),
        Some(key(KeyCode::Esc, KeyModifiers::NONE))
    );
    assert_eq!(
        bindings.translate(Scope::Completion, chord),
        Some(key(KeyCode::Esc, KeyModifiers::NONE))
    );
}

#[test]
fn global_bindings_cannot_silently_shadow_any_local_surface() {
    for text in [
        "[keybindings]\nquit = [\"esc\"]\n",
        "[keybindings]\nprompt-cancel = [\"ctrl+q\"]\n",
    ] {
        let error = parse(text).expect_err("global/local effective collision must fail");
        let message = error.to_string();
        assert!(message.contains("quit"), "{message}");
        assert!(message.contains("prompt-cancel"), "{message}");
        assert!(message.contains("prompt"), "{message}");
    }
}

#[test]
fn collisions_name_both_actions_and_normalized_chord() {
    let error = parse("[keybindings]\nsave = [\"control+W\"]\n")
        .expect_err("save collides with close default");
    let message = error.to_string();
    assert!(message.contains("save"), "{message}");
    assert!(message.contains("close"), "{message}");
    assert!(message.contains("ctrl+w"), "{message}");

    let error = parse("[keybindings]\nsave = [\"ctrl+shift+A\"]\nclose = [\"control+shift+a\"]\n")
        .expect_err("normalized user duplicates must fail");
    assert!(error.to_string().contains("save"));
    assert!(error.to_string().contains("close"));

    let error = parse("[keybindings]\nsave = [\"alt+s\"]\n\"ALT+S\" = \"open\"\n")
        .expect_err("legacy and action forms must not silently override each other");
    let message = error.to_string();
    assert!(message.contains("save"), "{message}");
    assert!(message.contains("open"), "{message}");
    assert!(message.contains("alt+s"), "{message}");
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
fn mouse_gestures_can_be_reassigned_or_unbound() {
    let bindings =
        parse("[keybindings]\nmouse-place-cursor = []\nmouse-select-word = [\"mouse-left\"]\n")
            .unwrap();
    assert_eq!(
        bindings.mouse_action(Scope::Editor, MouseGesture::Left),
        Some(Action::MouseSelectWord)
    );
    assert_eq!(
        bindings.mouse_action(Scope::Editor, MouseGesture::LeftDouble),
        None
    );
}

#[test]
fn wheel_gestures_can_be_swapped_or_unbound_without_crossing_button_types() {
    let bindings =
        parse("[keybindings]\nmouse-scroll-up = []\nmouse-scroll-down = [\"mouse-wheel-up\"]\n")
            .unwrap();
    assert_eq!(
        bindings.mouse_action(Scope::Help, MouseGesture::ScrollUp),
        Some(Action::MouseScrollDown)
    );
    assert_eq!(
        bindings.mouse_action(Scope::Editor, MouseGesture::ScrollDown),
        None
    );
    assert!(parse("[keybindings]\nmouse-place-cursor = [\"mouse-wheel-up\"]\n").is_err());
    assert!(parse("[keybindings]\nmouse-scroll-up = [\"mouse-left\"]\n").is_err());
}

#[test]
fn registry_defaults_are_complete_and_collision_free() {
    let bindings = KeyBindings::default();
    assert_eq!(actions::REGISTRY.len(), 83);
    for descriptor in actions::REGISTRY {
        assert!(!descriptor.name.is_empty());
        assert!(!descriptor.scopes.is_empty());
        assert!(!descriptor.defaults.is_empty());
    }
    assert!(!bindings.keys.is_empty());
}
