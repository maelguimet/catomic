//! Purpose: track the pointer affordance for detected links.
//! Owns: the hovered document range and focus cleanup.
//! Must not: detect URLs, open destinations, mutate buffers, or emit terminal protocols.
//! Invariants: unchanged hover emits no frame; losing focus clears the hover.

use std::io;

use crate::terminal::render::TextHighlight;

#[derive(Debug, Default)]
pub(crate) struct LinkInteractionState {
    hovered: Option<TextHighlight>,
}

impl LinkInteractionState {
    pub(crate) fn hovered(&self) -> Option<TextHighlight> {
        self.hovered
    }

    pub(crate) fn set_hovered(&mut self, hovered: Option<TextHighlight>) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        true
    }
}

pub(super) fn clear_on_focus_loss(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    if app.link_interaction.set_hovered(None) {
        app.render(out)?;
    }
    Ok(())
}
