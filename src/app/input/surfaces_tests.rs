use super::*;

const FOCUSED: [Surface; 9] = [
    Surface::Help,
    Surface::RecoveryPreview,
    Surface::CommandPreview,
    Surface::Replace,
    Surface::ReplaceReview,
    Surface::Search,
    Surface::Command,
    Surface::Picker,
    Surface::MarkdownPreview,
];

fn position(stop: KeyStop) -> usize {
    KEY_ORDER
        .iter()
        .position(|candidate| *candidate == stop)
        .unwrap()
}

#[test]
fn every_focusable_surface_has_exactly_one_key_stop() {
    for surface in FOCUSED {
        let stops = KEY_ORDER
            .iter()
            .filter(|stop| **stop == KeyStop::Focus(surface))
            .count();
        assert_eq!(stops, 1, "{surface:?}");
    }
}

#[test]
fn workers_keep_their_canonical_place_around_the_focused_surface() {
    assert_eq!(KEY_ORDER[0], KeyStop::LintWorker);
    assert!(position(KeyStop::Focus(Surface::Help)) < position(KeyStop::CommandWorker));
    assert!(position(KeyStop::Focus(Surface::RecoveryPreview)) < position(KeyStop::CommandWorker));
    assert!(position(KeyStop::CommandWorker) < position(KeyStop::Focus(Surface::Replace)));
    assert!(position(KeyStop::Focus(Surface::Command)) < position(KeyStop::GotoWorker));
    assert!(position(KeyStop::GotoWorker) < position(KeyStop::Focus(Surface::Picker)));
    assert!(
        position(KeyStop::CompletionTrigger) < position(KeyStop::Focus(Surface::MarkdownPreview))
    );
}

#[test]
fn paste_under_a_mobile_overlay_is_not_routed_to_the_surface_beneath() {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();
    replace::open_prompt(&mut app, &mut out, false).unwrap();
    super::super::super::mobile::open_notice_for_test(&mut app, "notice");

    assert!(!handle_paste(&mut app, &mut out, "b").unwrap());
}
