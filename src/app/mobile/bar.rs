//! Purpose: compose compact, hit-testable mobile action rows for the active surface.
//! Owns: context-specific labels, cell spans, and width adaptation.
//! Must not: inspect App internals, dispatch actions, render ANSI, or mutate state.
//! Invariants: labels are ASCII cell widths; required cancel/accept paths fit at 20 columns.
//! Phase: Android/Termux mobile support.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Surface {
    Normal,
    Selection,
    TouchSelection,
    Prompt,
    Confirmation,
    ReadOnly,
    Running,
    Message,
    Menu,
    Notice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BarAction {
    Menu,
    Info,
    Cancel,
    Accept,
    Back,
    Up,
    Down,
    PageUp,
    PageDown,
    Save,
    Undo,
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ActionBar {
    pub(super) text: String,
    buttons: Vec<Button>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Button {
    start: usize,
    end: usize,
    action: BarAction,
}

impl ActionBar {
    pub(super) fn action_at(&self, column: usize) -> Option<BarAction> {
        self.buttons
            .iter()
            .find(|button| column >= button.start && column < button.end)
            .map(|button| button.action)
    }
}

pub(super) fn build(surface: Surface, width: usize) -> ActionBar {
    let specs: &[(&str, BarAction)] = match surface {
        Surface::Normal => &[
            ("[Menu]", BarAction::Menu),
            ("[Save]", BarAction::Save),
            ("[Undo]", BarAction::Undo),
        ],
        Surface::Selection => &[
            ("[Menu]", BarAction::Menu),
            ("[Copy]", BarAction::Copy),
            ("[Cut]", BarAction::Cut),
        ],
        Surface::TouchSelection | Surface::Running => &[("[Cancel]", BarAction::Cancel)],
        Surface::Message => &[
            ("[Menu]", BarAction::Menu),
            ("[Info]", BarAction::Info),
            ("[Save]", BarAction::Save),
        ],
        Surface::Prompt => &[
            ("[No]", BarAction::Cancel),
            ("[OK]", BarAction::Accept),
            ("[Up]", BarAction::Up),
            ("[Dn]", BarAction::Down),
        ],
        Surface::Confirmation => &[
            ("[Info]", BarAction::Info),
            ("[No]", BarAction::Cancel),
            ("[Yes]", BarAction::Accept),
            ("[Up]", BarAction::Up),
            ("[Dn]", BarAction::Down),
        ],
        Surface::ReadOnly => &[
            ("[Back]", BarAction::Back),
            ("[Up]", BarAction::Up),
            ("[Dn]", BarAction::Down),
            ("[PgUp]", BarAction::PageUp),
            ("[PgDn]", BarAction::PageDown),
        ],
        Surface::Menu => &[
            ("[Back]", BarAction::Back),
            ("[Up]", BarAction::Up),
            ("[Dn]", BarAction::Down),
            ("[Run]", BarAction::Accept),
        ],
        Surface::Notice => &[
            ("[Back]", BarAction::Back),
            ("[Up]", BarAction::Up),
            ("[Dn]", BarAction::Down),
        ],
    };
    compose(specs, width)
}

fn compose(specs: &[(&str, BarAction)], width: usize) -> ActionBar {
    let mut text = String::new();
    let mut buttons = Vec::new();
    for (label, action) in specs {
        if text.len().saturating_add(label.len()) > width {
            break;
        }
        let start = text.len();
        text.push_str(label);
        buttons.push(Button {
            start,
            end: text.len(),
            action: *action,
        });
    }
    ActionBar { text, buttons }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_width_keeps_essential_normal_and_confirmation_actions() {
        assert_eq!(build(Surface::Normal, 20).text, "[Menu][Save][Undo]");
        let confirmation = build(Surface::Confirmation, 20);
        assert_eq!(confirmation.text, "[Info][No][Yes][Up]");
        assert_eq!(confirmation.action_at(8), Some(BarAction::Cancel));
        assert_eq!(confirmation.action_at(13), Some(BarAction::Accept));
    }

    #[test]
    fn narrow_width_never_partially_renders_a_button() {
        assert_eq!(build(Surface::Normal, 10).text, "[Menu]");
        assert_eq!(build(Surface::Prompt, 8).text, "[No][OK]");
        assert_eq!(build(Surface::ReadOnly, 5).text, "");
    }

    #[test]
    fn transient_messages_keep_touch_details_and_save_recovery_reachable() {
        assert_eq!(build(Surface::Message, 20).text, "[Menu][Info][Save]");
    }
}
