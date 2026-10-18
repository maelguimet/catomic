//! Purpose: name the one surface that receives keys, paste, and mobile bar actions.
//! Owns: the Focus value and the state of whichever surface is focused.
//! Must not: own background workers, decode terminal events, render, or touch files.
//! Invariants: one surface is focused at a time; opening a surface closes the previous one;
//!   only a mobile overlay keeps the surface beneath it, and closing it restores that surface.

use super::{
    command_prompt, completion, external_command, help, mobile, recovery, replace, search, view,
};

#[derive(Default)]
pub(crate) enum Focus {
    #[default]
    Editor,
    Prompt(Prompt),
    Picker(Picker),
    ReadOnly(ReadOnlyView),
    Overlay(Box<mobile::Overlay>),
}

pub(crate) enum Prompt {
    Search(search::SearchPrompt),
    Replace(replace::ReplaceFlow),
    Command(command_prompt::ActivePrompt),
}

pub(crate) enum Picker {
    Word(completion::ActiveWordCompletion),
    Emoji(completion::ActiveEmojiPicker),
}

pub(crate) enum ReadOnlyView {
    Help(Box<help::HelpView>),
    RecoveryPreview(Box<recovery::RecoveryPreview>),
    CommandPreview(Box<external_command::CommandPreview>),
    MarkdownPreview(Box<view::PreviewDocument>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    Editor,
    Search,
    Replace,
    ReplaceReview,
    Command,
    Picker,
    Help,
    RecoveryPreview,
    CommandPreview,
    MarkdownPreview,
    MobileOverlay,
}

impl Focus {
    pub(crate) fn surface(&self) -> Surface {
        match self {
            Focus::Editor => Surface::Editor,
            Focus::Prompt(Prompt::Search(_)) => Surface::Search,
            Focus::Prompt(Prompt::Replace(flow)) if flow.is_review() => Surface::ReplaceReview,
            Focus::Prompt(Prompt::Replace(_)) => Surface::Replace,
            Focus::Prompt(Prompt::Command(_)) => Surface::Command,
            Focus::Picker(_) => Surface::Picker,
            Focus::ReadOnly(ReadOnlyView::Help(_)) => Surface::Help,
            Focus::ReadOnly(ReadOnlyView::RecoveryPreview(_)) => Surface::RecoveryPreview,
            Focus::ReadOnly(ReadOnlyView::CommandPreview(_)) => Surface::CommandPreview,
            Focus::ReadOnly(ReadOnlyView::MarkdownPreview(_)) => Surface::MarkdownPreview,
            Focus::Overlay(_) => Surface::MobileOverlay,
        }
    }

    pub(crate) fn base(&self) -> &Focus {
        match self {
            Focus::Overlay(overlay) => overlay.underneath().base(),
            focus => focus,
        }
    }

    pub(crate) fn base_mut(&mut self) -> &mut Focus {
        match self {
            Focus::Overlay(overlay) => overlay.underneath_mut().base_mut(),
            focus => focus,
        }
    }

    pub(crate) fn base_surface(&self) -> Surface {
        self.base().surface()
    }

    pub(crate) fn is_editor(&self) -> bool {
        matches!(self, Focus::Editor)
    }
}

macro_rules! focused {
    ($get:ident, $get_mut:ident, $ty:ty, $pattern:pat => $value:ident) => {
        impl Focus {
            pub(crate) fn $get(&self) -> Option<&$ty> {
                match self.base() {
                    $pattern => Some($value),
                    _ => None,
                }
            }

            pub(crate) fn $get_mut(&mut self) -> Option<&mut $ty> {
                match self.base_mut() {
                    $pattern => Some($value),
                    _ => None,
                }
            }
        }
    };
}

focused!(search, search_mut, search::SearchPrompt, Focus::Prompt(Prompt::Search(prompt)) => prompt);
focused!(replace, replace_mut, replace::ReplaceFlow, Focus::Prompt(Prompt::Replace(flow)) => flow);
focused!(command, command_mut, command_prompt::ActivePrompt, Focus::Prompt(Prompt::Command(prompt)) => prompt);
focused!(picker, picker_mut, Picker, Focus::Picker(picker) => picker);
focused!(help, help_mut, help::HelpView, Focus::ReadOnly(ReadOnlyView::Help(view)) => view);
focused!(recovery_preview, recovery_preview_mut, recovery::RecoveryPreview, Focus::ReadOnly(ReadOnlyView::RecoveryPreview(preview)) => preview);
focused!(command_preview, command_preview_mut, external_command::CommandPreview, Focus::ReadOnly(ReadOnlyView::CommandPreview(preview)) => preview);
focused!(markdown_preview, markdown_preview_mut, view::PreviewDocument, Focus::ReadOnly(ReadOnlyView::MarkdownPreview(preview)) => preview);

pub(super) fn take(app: &mut super::App, surface: Surface) -> Option<Focus> {
    let focus = app.focus.base_mut();
    (focus.surface() == surface).then(|| std::mem::take(focus))
}

pub(super) fn close(app: &mut super::App) {
    loop {
        let surface = app.focus.surface();
        match surface {
            Surface::Editor => return,
            Surface::MobileOverlay => {
                mobile::close_overlay(app);
            }
            Surface::Search => search::cancel_running_search(app),
            Surface::Replace | Surface::ReplaceReview => replace::cancel(app),
            Surface::Command => app.focus = Focus::Editor,
            Surface::Picker => {
                completion::cancel(app);
            }
            Surface::Help => help::close(app),
            Surface::RecoveryPreview => {
                recovery::close(app);
            }
            Surface::CommandPreview => {
                external_command::close_preview(app);
            }
            Surface::MarkdownPreview => view::cancel_preview(app),
        }
        if app.focus.surface() == surface {
            app.focus = Focus::Editor;
        }
    }
}
