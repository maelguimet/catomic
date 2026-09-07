use super::*;

#[test]
fn editing_moves_and_deletes_whole_graphemes_at_the_caret() {
    let mut prompt = PromptInput::default();
    assert!(prompt.insert("a\u{301}猫👩\u{200d}💻z"));
    prompt.action(Action::PromptHome);
    prompt.action(Action::PromptMoveRight);
    assert_eq!(prompt.caret(), "a\u{301}".len());
    prompt.insert("!");
    prompt.action(Action::PromptDeleteForward);
    assert_eq!(prompt.as_str(), "a\u{301}!👩\u{200d}💻z");
    prompt.action(Action::PromptEnd);
    prompt.action(Action::PromptMoveLeft);
    prompt.action(Action::PromptDeleteBackward);
    assert_eq!(prompt.as_str(), "a\u{301}!z");
    prompt.action(Action::PromptHome);
    prompt.action(Action::PromptDeleteForward);
    assert_eq!(prompt.as_str(), "!z");
}

#[test]
fn insertion_and_deletion_snap_across_newly_joined_clusters() {
    let mut prompt = PromptInput::default();
    prompt.insert("👩💻");
    prompt.action(Action::PromptMoveLeft);
    prompt.insert("\u{200d}");
    assert_eq!(prompt.caret(), prompt.as_str().len());
    prompt.action(Action::PromptDeleteBackward);
    assert!(prompt.is_empty());
    prompt.insert("🇫x🇷");
    prompt.action(Action::PromptHome);
    prompt.action(Action::PromptMoveRight);
    prompt.action(Action::PromptDeleteForward);
    assert_eq!(prompt.as_str(), "🇫🇷");
    assert_eq!(prompt.caret(), prompt.as_str().len());
}

#[test]
fn pasted_controls_are_literal_normalized_and_render_inertly() {
    let mut prompt = PromptInput::default();
    prompt.insert("ab");
    prompt.action(Action::PromptMoveLeft);
    prompt.insert("\r\n\t\x1b[31m\r");
    assert_eq!(prompt.as_str(), "a\n\t\x1b[31m\nb");
    let shown = prompt.presentation("Find", "", 80);
    assert_eq!(shown.text, "Find: a␊␉␛[31m␊b");
    assert_eq!(shown.caret_cell, Some(15));
}

#[test]
fn oversized_paste_is_rejected_atomically_and_reported() {
    let mut prompt = PromptInput::default();
    prompt.insert("safe");
    assert!(!prompt.insert(&"x".repeat(MAX_PROMPT_BYTES)));
    assert_eq!(prompt.as_str(), "safe");
    assert_eq!(prompt.caret(), 4);
    assert!(prompt.presentation("Find", "", 80).text.contains("16 KiB"));
}

#[test]
fn horizontal_clipping_keeps_caret_and_whole_graphemes_on_screen() {
    let mut prompt = PromptInput::default();
    prompt.insert("start-a\u{301}猫👩\u{200d}💻-middle-終わり");
    for width in 0..40 {
        prompt.action(Action::PromptHome);
        loop {
            let view = prompt.presentation("Open file", "", width);
            assert!(
                UnicodeWidthStr::width(view.text.as_str()) <= width,
                "{width}: {:?}",
                view.text
            );
            assert!(view.caret_cell.is_none_or(|cell| cell < width));
            if prompt.caret() == prompt.as_str().len() {
                break;
            }
            prompt.action(Action::PromptMoveRight);
        }
    }
    prompt.action(Action::PromptHome);
    let start = prompt.presentation("Find", "", 12);
    assert!(start.text.starts_with("Find: start"));
    assert_eq!(start.caret_cell, Some(6));
    prompt.action(Action::PromptEnd);
    let end = prompt.presentation("Find", "", 12);
    assert!(end.text.contains('…'));
    assert_eq!(end.caret_cell, Some(11));
}
