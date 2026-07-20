//! Purpose: prove inline discovery, privacy boundaries, strict delimiters, and response mapping.
//! Owns: pure Unicode/configured-marker/multi-block/limit fixtures without App or network.
//! Must not: perform I/O, construct clients, mutate buffers, or duplicate integration tests.
//! Invariants: asserted line numbers are one-based and context never includes unrelated text.

use super::response::ResponseError;
use super::*;
use crate::config::llm::{InlineBlockMode, InlineSettings};

#[test]
fn selection_wins_and_contains_only_exact_unicode_selection() {
    let document = ">> Rewrite this\nprivate\n猫🙂 target\n<catblock>\nignored\n<catblock>\n";
    let draft = discover(
        document,
        2,
        Some((Cursor { row: 2, col: 0 }, Cursor { row: 2, col: 2 })),
        None,
        &InlineSettings::default(),
    )
    .unwrap();

    assert_eq!(draft.scope, InlineScope::Selection);
    assert_eq!(draft.requests[0].text, "猫🙂");
    assert!(!draft.requests[0].text.contains("private"));
    assert_eq!(draft.instruction.display_line, 1);
}

#[test]
fn marker_matching_is_explicit_and_ambiguity_fails_closed() {
    assert_eq!(
        discover(
            ">>operator\ntext\n",
            1,
            None,
            None,
            &InlineSettings::default()
        ),
        Err(InlineError::MissingInstruction)
    );
    let settings = InlineSettings {
        instruction_prefix: ">>>".to_string(),
        ..InlineSettings::default()
    };
    assert_eq!(
        discover(
            ">>> catomic\nRewrite\n<<<\ntext\n",
            1,
            None,
            None,
            &settings
        ),
        Err(InlineError::AmbiguousInstruction { lines: vec![1, 1] })
    );
}

#[test]
fn one_block_strips_delimiters_and_unrelated_content() {
    let document =
        ">> Simplify this\nsecret outside\n<catblock>\nfn cat() {}\n</catblock>\nprivate after\n";
    let draft = discover(document, 3, None, None, &InlineSettings::default()).unwrap();

    assert_eq!(draft.scope, InlineScope::Blocks);
    assert_eq!(draft.targets[0].range.original, "fn cat() {}\n");
    assert_eq!(draft.requests[0].text, "fn cat() {}\n");
    assert!(!draft.requests[0].text.contains("catblock"));
    assert!(!draft.requests[0].text.contains("secret outside"));
    assert_eq!(draft.delimiter_guards.len(), 2);
}

#[test]
fn combined_blocks_have_stable_ids_boundaries_and_no_interstitial_text() {
    let document =
        ">> Rename cat\n<catblock>\none\n</catblock>\nDO NOT SEND\n<catblock>\ntwo\n</catblock>\n";
    let draft = discover(document, 1, None, None, &InlineSettings::default()).unwrap();

    assert_eq!(
        draft
            .targets
            .iter()
            .map(|target| target.id)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(draft.requests[0]
        .text
        .contains("[Context block 1 of 2; source lines 3-3]"));
    assert!(draft.requests[0].text.contains("[End context block 2]"));
    assert!(!draft.requests[0].text.contains("DO NOT SEND"));
}

#[test]
fn queued_blocks_are_serial_request_units_and_respect_the_limit() {
    let document = ">> Rename\n<catblock>\none\n</catblock>\n<catblock>\ntwo\n</catblock>\n";
    let mut settings = InlineSettings {
        block_mode: InlineBlockMode::Queued,
        ..InlineSettings::default()
    };
    let draft = discover(document, 0, None, None, &settings).unwrap();
    assert_eq!(draft.requests.len(), 2);
    assert_eq!(draft.requests[0].target_ids, [1]);
    assert_eq!(draft.requests[1].target_ids, [2]);

    settings.queue_limit = 1;
    assert_eq!(
        discover(document, 0, None, None, &settings),
        Err(InlineError::QueueLimit {
            blocks: 2,
            limit: 1
        })
    );
}

#[test]
fn malformed_delimiters_fail_closed_with_one_based_lines() {
    let settings = InlineSettings::default();
    assert_eq!(
        discover(">> Fix\n</catblock>\n", 0, None, None, &settings),
        Err(InlineError::UnexpectedContextClose { line: 2 })
    );
    assert_eq!(
        discover(">> Fix\n<catblock>\n<catblock>\n", 0, None, None, &settings),
        Err(InlineError::NestedContextOpen {
            line: 3,
            open_line: 2
        })
    );
    assert_eq!(
        discover(">> Fix\n<catblock>\ntext\n", 0, None, None, &settings),
        Err(InlineError::UnclosedContext { line: 2 })
    );
    assert_eq!(
        discover(
            ">> Fix\n<catblock>\n</catblock>\n",
            0,
            None,
            None,
            &settings
        ),
        Err(InlineError::EmptyContextBlock { line: 2 })
    );
}

#[test]
fn configurable_comment_marker_and_suffix_are_stripped() {
    let settings = InlineSettings {
        instruction_prefix: "<!-- >>".to_string(),
        instruction_suffix: "-->".to_string(),
        ..InlineSettings::default()
    };
    let draft = discover(
        "<!-- >> Rewrite accessibly -->\n<catblock>\n<h1>Cat</h1>\n</catblock>\n",
        2,
        None,
        None,
        &settings,
    )
    .unwrap();
    assert_eq!(draft.instruction.text, "Rewrite accessibly");
    assert!(!draft.requests[0].text.contains("<!--"));
}

#[test]
fn nearest_preceding_marker_is_deterministic_and_selection_cannot_edit_it() {
    let document = ">> First\none\n>> Second\ntwo\n";
    let draft = discover(document, 3, None, None, &InlineSettings::default()).unwrap();
    assert_eq!(draft.instruction.text, "Second");
    assert_eq!(draft.instruction.display_line, 3);

    assert_eq!(
        discover(
            document,
            2,
            Some((Cursor { row: 2, col: 0 }, Cursor { row: 3, col: 3 })),
            None,
            &InlineSettings::default(),
        ),
        Err(InlineError::SelectionContainsInstruction { line: 3 })
    );
}

#[test]
fn legacy_instruction_blocks_remain_discoverable_and_are_metadata() {
    let document = ">>> catomic\nRewrite this safely.\n<<<\n<catblock>\ncode\n</catblock>\n";
    let draft = discover(document, 1, None, None, &InlineSettings::default()).unwrap();
    assert_eq!(draft.instruction.text, "Rewrite this safely.");
    assert!(draft.instruction.legacy_block);
    assert_eq!(draft.requests[0].text, "code\n");
}

#[test]
fn full_file_fallback_replaces_instruction_metadata_with_a_sentinel() {
    let document = "alpha\n>> Rewrite all\nomega\n";
    let draft = discover(document, 2, None, None, &InlineSettings::default()).unwrap();
    assert_eq!(draft.scope, InlineScope::FullFile);
    assert!(!draft.requests[0].text.contains(">> Rewrite all"));
    assert!(draft.requests[0]
        .text
        .contains("CATOMIC-INSTRUCTION-METADATA"));
    assert_eq!(draft.full_file_lines, 4);
    assert_eq!(draft.full_file_bytes, document.len());
}

#[test]
fn instruction_metadata_and_original_full_file_cannot_bypass_hard_limits() {
    let oversized_instruction = format!(">> {}\nselected\n", "x".repeat(64 * 1024 + 1));
    assert!(matches!(
        discover(
            &oversized_instruction,
            1,
            Some((Cursor { row: 1, col: 0 }, Cursor { row: 1, col: 8 })),
            None,
            &InlineSettings::default(),
        ),
        Err(InlineError::Context(ContextError::TooLarge { .. }))
    ));

    let oversized_file = format!(">> Rewrite\n{}", "x".repeat(64 * 1024));
    assert!(matches!(
        discover(&oversized_file, 1, None, None, &InlineSettings::default()),
        Err(InlineError::Context(ContextError::TooLarge { .. }))
    ));
}

#[test]
fn combined_response_requires_each_expected_id_exactly_once() {
    let parsed = parse_combined_replacements(
        r#"{"catomic_replacements":[{"block":2,"replacement":"two"},{"block":1,"replacement":"猫"}]}"#,
        &[1, 2],
    )
    .unwrap();
    assert_eq!(parsed.get(&1).map(String::as_str), Some("猫"));
    assert_eq!(parsed.get(&2).map(String::as_str), Some("two"));

    assert_eq!(
        parse_combined_replacements(
            r#"{"catomic_replacements":[{"block":1,"replacement":"one"}]}"#,
            &[1, 2],
        ),
        Err(ResponseError::MissingBlock { id: 2 })
    );
    assert_eq!(
        parse_combined_replacements(
            r#"{"catomic_replacements":[{"block":1,"replacement":"a"},{"block":1,"replacement":"b"}]}"#,
            &[1],
        ),
        Err(ResponseError::DuplicateBlock { id: 1 })
    );
}
