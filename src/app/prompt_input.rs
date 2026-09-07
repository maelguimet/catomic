//! Shared, bounded prompt editing and immutable terminal-cell presentation.
//! Carets are UTF-8 byte offsets at extended grapheme boundaries. Pasted controls
//! remain literal data and render as visible glyphs, never terminal instructions.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::config::actions::Action;
use crate::editor::text_layout;

// Bounds string movement, segmentation and layout on every interactive event.
pub(super) const MAX_PROMPT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default)]
pub(super) struct PromptInput {
    text: String,
    caret: usize,
    notice: Option<String>,
}

pub(super) struct PromptPresentation {
    pub(super) text: String,
    /// Zero-based terminal cell; None for a zero-width prompt.
    pub(super) caret_cell: Option<usize>,
}

impl PromptInput {
    pub(super) fn as_str(&self) -> &str {
        &self.text
    }

    pub(super) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub(super) fn caret(&self) -> usize {
        self.caret
    }

    pub(super) fn notice(&mut self, notice: impl Into<String>) {
        self.notice = Some(notice.into());
    }

    pub(super) fn insert(&mut self, text: &str) -> bool {
        if text.len().saturating_add(self.text.len()) > MAX_PROMPT_BYTES {
            self.notice("Prompt limit: 16 KiB; input was not inserted.");
            return false;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() {
            return false;
        }
        self.notice = None;
        self.text.insert_str(self.caret, &text);
        self.caret += text.len();
        self.snap_caret_forward();
        true
    }

    /// Replace a completed path component without disturbing its suffix.
    pub(super) fn replace(&mut self, start: usize, end: usize, text: &str) -> bool {
        if self.text.len() - (end - start) + text.len() > MAX_PROMPT_BYTES {
            self.notice("Prompt limit: 16 KiB; completion was not inserted.");
            return false;
        }
        self.text.replace_range(start..end, text);
        self.caret = start + text.len();
        self.snap_caret_forward();
        self.notice = None;
        true
    }

    fn snap_caret_forward(&mut self) {
        // Inserting a combining mark or joiner can merge with the following
        // cluster. Never leave the caret inside that newly formed grapheme.
        self.caret = self
            .text
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .find(|&byte| byte >= self.caret)
            .unwrap_or(self.text.len());
    }

    /// None means not a text-editing action; Some says whether content changed.
    pub(super) fn action(&mut self, action: Action) -> Option<bool> {
        if !is_edit_action(action) {
            return None;
        }
        self.notice = None;
        let previous = || {
            self.text[..self.caret]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(byte, _)| byte)
        };
        let next = || {
            self.text[self.caret..]
                .graphemes(true)
                .next()
                .map_or(self.caret, |g| self.caret + g.len())
        };
        match action {
            Action::PromptMoveLeft => self.caret = previous(),
            Action::PromptMoveRight => self.caret = next(),
            Action::PromptHome => self.caret = 0,
            Action::PromptEnd => self.caret = self.text.len(),
            Action::PromptDeleteBackward if self.caret > 0 => {
                let start = previous();
                self.text.replace_range(start..self.caret, "");
                self.caret = start;
                self.snap_caret_forward();
                return Some(true);
            }
            Action::PromptDeleteForward if self.caret < self.text.len() => {
                self.text.replace_range(self.caret..next(), "");
                self.snap_caret_forward();
                return Some(true);
            }
            _ => {}
        }
        Some(false)
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Option<bool> {
        let action = match key.code {
            KeyCode::Left => Action::PromptMoveLeft,
            KeyCode::Right => Action::PromptMoveRight,
            KeyCode::Home => Action::PromptHome,
            KeyCode::End => Action::PromptEnd,
            KeyCode::Backspace => Action::PromptDeleteBackward,
            KeyCode::Delete => Action::PromptDeleteForward,
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                let ch = if key.modifiers.contains(KeyModifiers::SHIFT) && ch.is_ascii_lowercase() {
                    ch.to_ascii_uppercase()
                } else {
                    ch
                };
                return Some(self.insert(ch.encode_utf8(&mut [0; 4])));
            }
            _ => return None,
        };
        self.action(action)
    }

    pub(super) fn presentation(&self, label: &str, hint: &str, width: usize) -> PromptPresentation {
        if width == 0 {
            return PromptPresentation {
                text: String::new(),
                caret_cell: None,
            };
        }
        let label = if let Some(notice) = self.notice.as_deref() {
            text_layout::terminal_safe_clipped(&format!("{notice}: "), width.saturating_sub(4))
        } else {
            let label_budget = width
                .saturating_sub(16)
                .max(width / 2)
                .min(width.saturating_sub(4));
            text_layout::terminal_safe_tail_clipped(&format!("{label}: "), label_budget)
        };
        let label_cells = UnicodeWidthStr::width(label.as_str());
        let available = width - label_cells;
        let mut safe_text = String::with_capacity(self.text.len());
        let graphemes: Vec<_> = self
            .text
            .grapheme_indices(true)
            .map(|(byte, g)| {
                let start = safe_text.len();
                safe_text.extend(g.chars().map(text_layout::terminal_safe_char));
                if UnicodeWidthStr::width(&safe_text[start..]) == 0 {
                    safe_text.insert(start, '◌');
                }
                let cells = UnicodeWidthStr::width(&safe_text[start..]);
                (byte, start..safe_text.len(), cells)
            })
            .collect();
        let caret_index = graphemes.partition_point(|(byte, _, _)| *byte < self.caret);
        let before_cells: usize = graphemes[..caret_index].iter().map(|g| g.2).sum();
        let clipped_left = before_cells >= available;
        let marker = usize::from(clipped_left && available >= 3);
        let mut start = caret_index;
        let mut cells = 0;
        while start > 0 && cells + graphemes[start - 1].2 < available - marker {
            start -= 1;
            cells += graphemes[start].2;
        }
        let mut text = label;
        if marker != 0 {
            text.push('…');
        }
        let caret_cell = label_cells + marker + cells;
        let mut used = marker;
        let mut end = start;
        for (_, range, cells) in &graphemes[start..] {
            if used + cells > available {
                break;
            }
            text.push_str(&safe_text[range.clone()]);
            used += cells;
            end += 1;
        }
        if end < graphemes.len() && used < available {
            text.push('…');
            used += 1;
        }
        let hint = if self.notice.is_some() { "" } else { hint };
        if !hint.is_empty() && end == graphemes.len() && available - used >= 4 {
            text.push_str(&text_layout::terminal_safe_clipped(
                &format!("  {hint}"),
                available - used,
            ));
        }
        PromptPresentation {
            text,
            caret_cell: Some(caret_cell),
        }
    }
}

pub(super) fn is_edit_action(action: Action) -> bool {
    matches!(
        action,
        Action::PromptMoveLeft
            | Action::PromptMoveRight
            | Action::PromptHome
            | Action::PromptEnd
            | Action::PromptDeleteBackward
            | Action::PromptDeleteForward
    )
}

#[cfg(test)]
mod tests;
