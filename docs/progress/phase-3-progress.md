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

## Remaining Phase 3 Work

- Forward/backward navigation from the current match across descriptor-backed
  paged files (incremental lookup currently returns the first whole-file match).
- Goto line and a basic command surface.
- Selection expansion, mouse click/drag, and selection cut/copy/paste.
- Phase 3 acceptance definition, medium-file search measurement, and manual UX
  checklist evidence.
