# Phase 3 Progress

Phase 3 is in progress. Phase 2 acceptance remains the prerequisite and is
recorded in [`../phase-2-acceptance.md`](../phase-2-acceptance.md).

## Completed

- **3-a incremental search foundation**: Ctrl+F now searches while the prompt
  changes, moves the cursor to the live match, and reverse-highlights that match
  in the visible viewport. Enter/Down and Up navigate forward/backward with
  wraparound in ordinary PieceTable buffers; Escape closes the search state.
- Unicode match positions use the editor's current scalar-column model.
- Oversized paged files start a fresh cancellable bounded descriptor search as
  the query changes, retaining unsaved-page overlays and cross-page matches.
- Existing real PTY Ctrl+F coverage remains green, including quitting while the
  search prompt is active.
- **3-b paged search navigation**: Enter/Down and Up now select relative to the
  current descriptor match and wrap across logical file pages. The bounded
  scanner still honors edited-page overlays and cross-boundary matches.
- **3-c command foundation**: Ctrl+G provides a cancellable 1-based goto prompt
  across ordinary and descriptor-backed paged buffers, including edited-page
  overlays. Ctrl+Shift+P accepts `goto N`, `save`/`write`, and `quit`/`q`,
  reusing the existing conflict-safe save and dirty-buffer quit paths.
- **3-d keyboard selection and clipboard**: Shift+arrows and Ctrl+A create
  multiline scalar-coordinate selections. Ctrl+C/X/V use an internal clipboard,
  copy exports OSC 52 when bounded, and bracketed paste plus selection replacement
  are each recorded as one piece-level undo transaction.
- **3-e mouse selection**: terminal mouse capture maps clicks through both
  viewport offsets, left-button drags create multiline selections, status-row
  clicks are ignored, and bounded double clicks expand Unicode word or
  punctuation runs.

## Remaining Phase 3 Work

- Phase 3 acceptance definition, medium-file search measurement, and manual UX
  checklist evidence.
