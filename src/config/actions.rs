//! Purpose: define every stable, user-facing shortcut action and its surface contract.
//! Owns: action names, help text, scopes, and the default chord inventory.
//! Must not: parse user TOML, dispatch App behavior, inspect terminal events, or mutate state.
//! Invariants: names are unique; every action has a scope and at least one default chord.

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Scope {
    Global,
    Editor,
    Prompt,
    Search,
    Completion,
    Preview,
    Picker,
    Help,
}

impl Scope {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Editor => "editor",
            Self::Prompt => "prompt",
            Self::Search => "search",
            Self::Completion => "completion",
            Self::Preview => "preview",
            Self::Picker => "picker",
            Self::Help => "help",
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum InputKind {
    Keyboard,
    MouseButton,
    MouseWheel,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Action {
    Help,
    Quit,
    Interrupt,
    Save,
    SaveAs,
    Open,
    New,
    Close,
    Reload,
    Search,
    Replace,
    GotoLine,
    CommandPrompt,
    Complete,
    Undo,
    Redo,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    LineStart,
    LineEnd,
    SelectLineStart,
    SelectLineEnd,
    DocumentStart,
    DocumentEnd,
    SelectDocumentStart,
    SelectDocumentEnd,
    ViewportUp,
    ViewportDown,
    SelectViewportUp,
    SelectViewportDown,
    WordLeft,
    WordRight,
    SelectWordLeft,
    SelectWordRight,
    ParagraphPrevious,
    ParagraphNext,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    InsertNewline,
    Indent,
    Unindent,
    ToggleOverwrite,
    SelectAll,
    Copy,
    Cut,
    CutLine,
    Paste,
    PreviousBuffer,
    NextBuffer,
    PreviousPage,
    NextPage,
    MarkdownPreview,
    ToggleExternalDiff,
    LineNumbers,
    Whitespace,
    SoftWrap,
    SelectModel,
    RunClanker,
    ClearClankerChanges,
    PromptSubmit,
    PromptCancel,
    PromptDeleteBackward,
    SearchNext,
    SearchPrevious,
    SearchCancel,
    CompletionNext,
    CompletionPrevious,
    CompletionAccept,
    CompletionCancel,
    PreviewAccept,
    PreviewCancel,
    PickerAccept,
    PickerCancel,
    HelpClose,
    MousePlaceCursor,
    MouseExtendSelection,
    MouseFinishSelection,
    MouseSelectWord,
    MouseScrollUp,
    MouseScrollDown,
}

#[derive(Clone, Copy)]
pub(crate) struct Descriptor {
    pub(crate) action: Action,
    pub(crate) name: &'static str,
    pub(crate) help: &'static str,
    pub(crate) scopes: &'static [Scope],
    pub(crate) defaults: &'static [&'static str],
    pub(crate) input: InputKind,
}

mod registry;
pub(crate) use registry::REGISTRY;

pub(crate) fn descriptor(action: Action) -> &'static Descriptor {
    REGISTRY
        .iter()
        .find(|entry| entry.action == action)
        .expect("every Action must have a descriptor")
}

pub(crate) fn parse_action(name: &str) -> Option<Action> {
    let name = name.trim().to_ascii_lowercase();
    REGISTRY
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.action)
}

pub(crate) fn display_chord(raw: &str) -> String {
    if raw.starts_with("mouse-") {
        return raw.to_string();
    }
    raw.split('+')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "pageup" => "PageUp".to_string(),
            "pagedown" => "PageDown".to_string(),
            "backspace" => "Backspace".to_string(),
            "delete" => "Delete".to_string(),
            "insert" => "Insert".to_string(),
            "enter" => "Enter".to_string(),
            "esc" => "Esc".to_string(),
            "space" => "Space".to_string(),
            "tab" => "Tab".to_string(),
            "left" => "Left".to_string(),
            "right" => "Right".to_string(),
            "up" => "Up".to_string(),
            "down" => "Down".to_string(),
            "home" => "Home".to_string(),
            "end" => "End".to_string(),
            key if key.starts_with('f') => key.to_ascii_uppercase(),
            key => key.to_ascii_uppercase(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_user_guide_inventory_matches_registry() {
        let mut names = std::collections::HashSet::new();
        let guide = include_str!("../../docs/user-guide.md");
        let inventory = guide
            .split_once("<!-- action-registry-start -->")
            .and_then(|(_, tail)| tail.split_once("<!-- action-registry-end -->"))
            .map(|(inventory, _)| inventory)
            .expect("user guide action registry markers")
            .trim()
            .strip_prefix("```text\n")
            .and_then(|inventory| inventory.strip_suffix("\n```"))
            .expect("user guide action registry text fence");
        for descriptor in REGISTRY {
            assert!(names.insert(descriptor.name), "duplicate action name");
            assert!(
                !descriptor.help.trim().is_empty(),
                "{} must have help text",
                descriptor.name
            );
        }
        assert_eq!(inventory, registry_reference());
    }

    #[test]
    fn config_template_inventory_matches_registry_and_is_valid() {
        let template = include_str!("config_template.toml");
        let inventory = template
            .split_once("# action-registry-start\n")
            .and_then(|(_, tail)| tail.split_once("# action-registry-end"))
            .map(|(inventory, _)| inventory)
            .expect("config template action registry markers");
        let entries = inventory
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| line.contains(" = ["))
            .collect::<Vec<_>>();
        let documented = format!("[keybindings]\n{}\n", entries.join("\n"));
        let table = documented
            .parse::<toml::Table>()
            .expect("documented keybindings must be valid TOML")
            .remove("keybindings")
            .and_then(|value| value.as_table().cloned())
            .expect("documented keybindings table");

        assert_eq!(table.len(), REGISTRY.len(), "config action count");
        for descriptor in REGISTRY {
            let defaults = table
                .get(descriptor.name)
                .unwrap_or_else(|| panic!("missing config action {}", descriptor.name))
                .as_array()
                .expect("config action defaults must be an array")
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .expect("config action defaults must be strings")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                defaults, descriptor.defaults,
                "{} defaults",
                descriptor.name
            );
        }
        crate::config::validate_text(&documented)
            .expect("config check must accept the complete documented inventory");

        let replacement_note =
            "Uncommenting an action replaces its complete built-in default list; [] unbinds it.";
        assert_eq!(template.matches(replacement_note).count(), 1);
        assert!(template.contains(
            "# action-registry-start\n# Generated by `catomic config refresh-keybindings`"
        ));
    }

    fn registry_reference() -> String {
        REGISTRY
            .iter()
            .map(|descriptor| {
                format!(
                    "{} | {} | {}",
                    descriptor.name,
                    descriptor
                        .scopes
                        .iter()
                        .map(|scope| scope.name())
                        .collect::<Vec<_>>()
                        .join(","),
                    descriptor.defaults.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
