//! Purpose: define public editor actions, default bindings, and prompt-command help.
//! Owns: static names, aliases, default keys, concise purposes, and lookup metadata.
//! Must not: dispatch actions, read configuration, mutate editor state, or start services.
//! Invariants: normal-mode action shortcuts and parsed prompt commands come from this catalog;
//!   fixed editing/navigation keys are explicitly classified in `FIXED_SHORTCUTS`.
//! Phase: post-v0.1 discoverability and help-drift prevention.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const CTRL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;
const ALL_MODIFIERS: u8 = CTRL | SHIFT | ALT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorAction {
    Help,
    Quit,
    Save,
    SaveAs,
    Reload,
    Open,
    New,
    Close,
    Search,
    Replace,
    GotoLine,
    CommandPrompt,
    Undo,
    Redo,
    ToggleOverwrite,
    Complete,
    PreviousBuffer,
    NextBuffer,
    PreviousPage,
    NextPage,
    MarkdownPreview,
    LineNumbers,
    Whitespace,
    SoftWrap,
    SelectModel,
}

#[derive(Clone, Copy)]
enum ShortcutCode {
    Char(char),
    Null,
    Insert,
    PageUp,
    PageDown,
    Function(u8),
}

#[derive(Clone, Copy)]
pub(crate) struct ShortcutKey {
    code: ShortcutCode,
    required: u8,
    forbidden: u8,
}

impl ShortcutKey {
    const fn plain(code: ShortcutCode) -> Self {
        Self::new(code, 0, ALL_MODIFIERS)
    }

    const fn any(code: ShortcutCode) -> Self {
        Self::new(code, 0, 0)
    }

    const fn ctrl(code: ShortcutCode) -> Self {
        Self::new(code, CTRL, SHIFT | ALT)
    }

    const fn ctrl_required(code: ShortcutCode) -> Self {
        Self::new(code, CTRL, 0)
    }

    const fn ctrl_shift(code: ShortcutCode) -> Self {
        Self::new(code, CTRL | SHIFT, 0)
    }

    const fn alt_required(code: ShortcutCode) -> Self {
        Self::new(code, ALT, 0)
    }

    const fn new(code: ShortcutCode, required: u8, forbidden: u8) -> Self {
        Self {
            code,
            required,
            forbidden,
        }
    }

    fn matches(self, key: KeyEvent) -> bool {
        let modifiers = modifier_bits(key.modifiers);
        code_matches(self.code, key.code)
            && modifiers & self.required == self.required
            && modifiers & self.forbidden == 0
    }

    fn event(self) -> KeyEvent {
        let code = match self.code {
            ShortcutCode::Char(ch) => KeyCode::Char(ch),
            ShortcutCode::Null => KeyCode::Null,
            ShortcutCode::Insert => KeyCode::Insert,
            ShortcutCode::PageUp => KeyCode::PageUp,
            ShortcutCode::PageDown => KeyCode::PageDown,
            ShortcutCode::Function(number) => KeyCode::F(number),
        };
        let mut modifiers = KeyModifiers::NONE;
        if self.required & CTRL != 0 {
            modifiers.insert(KeyModifiers::CONTROL);
        }
        if self.required & SHIFT != 0 {
            modifiers.insert(KeyModifiers::SHIFT);
        }
        if self.required & ALT != 0 {
            modifiers.insert(KeyModifiers::ALT);
        }
        KeyEvent::new(code, modifiers)
    }
}

fn modifier_bits(modifiers: KeyModifiers) -> u8 {
    (u8::from(modifiers.contains(KeyModifiers::CONTROL)) * CTRL)
        | (u8::from(modifiers.contains(KeyModifiers::SHIFT)) * SHIFT)
        | (u8::from(modifiers.contains(KeyModifiers::ALT)) * ALT)
}

fn code_matches(expected: ShortcutCode, actual: KeyCode) -> bool {
    match (expected, actual) {
        (ShortcutCode::Char(expected), KeyCode::Char(actual)) => {
            expected.eq_ignore_ascii_case(&actual)
        }
        (ShortcutCode::Null, KeyCode::Null) => true,
        (ShortcutCode::Insert, KeyCode::Insert) => true,
        (ShortcutCode::PageUp, KeyCode::PageUp) => true,
        (ShortcutCode::PageDown, KeyCode::PageDown) => true,
        (ShortcutCode::Function(expected), KeyCode::F(actual)) => expected == actual,
        _ => false,
    }
}

pub(crate) struct EditorActionSpec {
    pub(crate) action: EditorAction,
    #[cfg(test)]
    pub(crate) name: &'static str,
    #[cfg(test)]
    pub(crate) category: &'static str,
    #[cfg(test)]
    pub(crate) default_keys: &'static str,
    #[cfg(test)]
    pub(crate) purpose: &'static str,
    bindings: &'static [ShortcutKey],
}

const fn action(
    action: EditorAction,
    _name: &'static str,
    _category: &'static str,
    _default_keys: &'static str,
    _purpose: &'static str,
    bindings: &'static [ShortcutKey],
) -> EditorActionSpec {
    EditorActionSpec {
        action,
        #[cfg(test)]
        name: _name,
        #[cfg(test)]
        category: _category,
        #[cfg(test)]
        default_keys: _default_keys,
        #[cfg(test)]
        purpose: _purpose,
        bindings,
    }
}

const CHAR_H: ShortcutCode = ShortcutCode::Char('h');

pub(crate) const EDITOR_ACTIONS: &[EditorActionSpec] = &[
    action(
        EditorAction::Help,
        "help",
        "Files and app",
        "Ctrl+H / F1",
        "Toggle this read-only help.",
        &[
            ShortcutKey::ctrl_required(CHAR_H),
            ShortcutKey::any(ShortcutCode::Function(1)),
        ],
    ),
    action(
        EditorAction::Quit,
        "quit",
        "Files and app",
        "Ctrl+Q",
        "Guarded quit; press again to discard all unsaved buffers.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('q'))],
    ),
    action(
        EditorAction::Save,
        "save",
        "Files and app",
        "Ctrl+S",
        "Save the active buffer; repeat only to confirm an unchanged disk conflict.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('s'))],
    ),
    action(
        EditorAction::SaveAs,
        "save-as",
        "Files and app",
        "Ctrl+Shift+S",
        "Choose a path; an existing destination requires unchanged repeat confirmation.",
        &[ShortcutKey::ctrl_shift(ShortcutCode::Char('s'))],
    ),
    action(
        EditorAction::Reload,
        "reload",
        "Files and app",
        "Ctrl+R",
        "Check disk state; repeat only to confirm reloading the same observed revision.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('r'))],
    ),
    action(
        EditorAction::Open,
        "open",
        "Files and app",
        "Ctrl+O",
        "Open a path in a buffer; a missing path is created only when saved.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('o'))],
    ),
    action(
        EditorAction::New,
        "new",
        "Files and app",
        "Ctrl+N",
        "Create a new untitled buffer.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('n'))],
    ),
    action(
        EditorAction::Close,
        "close",
        "Files and app",
        "Ctrl+W",
        "Close the active clean buffer; dirty buffers are refused.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('w'))],
    ),
    action(
        EditorAction::PreviousBuffer,
        "previous-buffer",
        "Buffers and pages",
        "Alt+PageUp",
        "Switch to the previous buffer.",
        &[ShortcutKey::alt_required(ShortcutCode::PageUp)],
    ),
    action(
        EditorAction::NextBuffer,
        "next-buffer",
        "Buffers and pages",
        "Alt+PageDown",
        "Switch to the next buffer.",
        &[ShortcutKey::alt_required(ShortcutCode::PageDown)],
    ),
    action(
        EditorAction::PreviousPage,
        "previous-page",
        "Buffers and pages",
        "Ctrl+PageUp",
        "Load the previous editable large-file page.",
        &[ShortcutKey::ctrl_required(ShortcutCode::PageUp)],
    ),
    action(
        EditorAction::NextPage,
        "next-page",
        "Buffers and pages",
        "Ctrl+PageDown",
        "Load the next editable large-file page.",
        &[ShortcutKey::ctrl_required(ShortcutCode::PageDown)],
    ),
    action(
        EditorAction::Search,
        "search",
        "Find and tools",
        "Ctrl+F",
        "Open incremental find; Enter/Down and Up move between matches.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('f'))],
    ),
    action(
        EditorAction::Replace,
        "replace",
        "Find and tools",
        "Ctrl+Shift+F",
        "Open the two-stage Replace Next prompt.",
        &[ShortcutKey::ctrl_shift(ShortcutCode::Char('f'))],
    ),
    action(
        EditorAction::GotoLine,
        "goto-line",
        "Find and tools",
        "Ctrl+G",
        "Go to a 1-based line number.",
        &[ShortcutKey::ctrl_required(ShortcutCode::Char('g'))],
    ),
    action(
        EditorAction::CommandPrompt,
        "command-prompt",
        "Find and tools",
        "Ctrl+Shift+P / F2",
        "Open the prompt for the commands listed below.",
        &[
            ShortcutKey::ctrl_shift(ShortcutCode::Char('p')),
            ShortcutKey::plain(ShortcutCode::Function(2)),
        ],
    ),
    action(
        EditorAction::Complete,
        "complete",
        "Find and tools",
        "Ctrl+Space",
        "Request bounded current-context completion.",
        &[
            ShortcutKey::ctrl_required(ShortcutCode::Char(' ')),
            ShortcutKey::ctrl_required(ShortcutCode::Null),
        ],
    ),
    action(
        EditorAction::Undo,
        "undo",
        "Editing",
        "Ctrl+Z",
        "Undo the last edit transaction.",
        &[ShortcutKey::ctrl(ShortcutCode::Char('z'))],
    ),
    action(
        EditorAction::Redo,
        "redo",
        "Editing",
        "Ctrl+Y / Ctrl+Shift+Z",
        "Redo the next edit transaction.",
        &[
            ShortcutKey::ctrl_required(ShortcutCode::Char('y')),
            ShortcutKey::new(ShortcutCode::Char('z'), CTRL | SHIFT, ALT),
        ],
    ),
    action(
        EditorAction::ToggleOverwrite,
        "toggle-overwrite",
        "Editing",
        "Insert",
        "Toggle session-wide insert/overwrite typing; overwrite replaces one grapheme.",
        &[ShortcutKey::plain(ShortcutCode::Insert)],
    ),
    action(
        EditorAction::MarkdownPreview,
        "markdown-preview",
        "View",
        "F6",
        "Toggle the read-only Markdown preview for Markdown buffers.",
        &[ShortcutKey::any(ShortcutCode::Function(6))],
    ),
    action(
        EditorAction::LineNumbers,
        "line-numbers",
        "View",
        "F7",
        "Toggle line numbers for all buffers and remember the explicit choice.",
        &[ShortcutKey::any(ShortcutCode::Function(7))],
    ),
    action(
        EditorAction::Whitespace,
        "whitespace",
        "View",
        "F8",
        "Toggle visible space and tab markers.",
        &[ShortcutKey::any(ShortcutCode::Function(8))],
    ),
    action(
        EditorAction::SoftWrap,
        "soft-wrap",
        "View",
        "F9",
        "Toggle visual wrapping without inserting newlines.",
        &[ShortcutKey::any(ShortcutCode::Function(9))],
    ),
    action(
        EditorAction::SelectModel,
        "select-model",
        "Models",
        "F10",
        "Open the searchable session model/backend picker without invoking a backend.",
        &[ShortcutKey::plain(ShortcutCode::Function(10))],
    ),
];

#[cfg(test)]
pub(crate) fn editor_action(name: &str) -> Option<EditorAction> {
    EDITOR_ACTIONS
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(name.trim()))
        .map(|spec| spec.action)
}

pub(crate) fn default_editor_action(key: KeyEvent) -> Option<EditorAction> {
    EDITOR_ACTIONS
        .iter()
        .find(|spec| spec.bindings.iter().any(|binding| binding.matches(key)))
        .map(|spec| spec.action)
}

pub(crate) fn canonical_key(action: EditorAction) -> KeyEvent {
    EDITOR_ACTIONS
        .iter()
        .find(|spec| spec.action == action)
        .and_then(|spec| spec.bindings.first())
        .expect("every editor action has a default binding")
        .event()
}

mod prompt_commands;
pub(crate) use prompt_commands::{prompt_command, PromptCommand, PROMPT_COMMANDS};

#[cfg(test)]
mod tests;
