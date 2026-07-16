# Phase 2 Progress Notes (b-ax) — Archived

**Purpose**: Archive of detailed completed Phase 2 sub-phase progress notes (2-b through 2-ax).
**Archived during**: Phase 2-q narrow cleanup (2026-06-22).
**Reason**: Keep TODO.md under the AGENTS.md "Over 800 lines: split before adding more" threshold.
**Moved from**: The "**Current status** (2026-06):" section at the end of TODO.md.
**Status**: Historical record only. Content preserved verbatim. No behavior or code implications.

Completed Phase 2 progress is archived here. See TODO.md for:
- current active Phase 2 goals / exit criteria
- the compact pointer to this archive
- the latest unresolved limitations that still matter

---

**Archived content (verbatim from TODO.md):**

- Phase 2-b (dirty quit guard + minimal message state) in progress / foundation:
  - App has message: Option<String>, pending_quit_confirm: bool.
  - Ctrl+Q: clean quits; dirty first press sets pending + short warning message (no quit); second press quits.
  - Actual content edits clear pending_quit_confirm and message (movement does not).
  - Successful Ctrl+S: clears dirty, pending, and message.
  - Save error via atomic: keeps dirty=true, sets "Save error: ..." message; no panic.
  - First dirty Ctrl+Q sets pending+message and visibly renders the warning (bottom row); clean Q quits immediately.
  - Minimal render bottom line: shows message text when present (reserves last row, no colors/UI redesign).
  - Tests cover all required cases (clean Q, dirty sequences, save clear, error keep-dirty+msg, edit clears pending+message, explicit temp lifecycle, render msg emission).
  - Limitation: Quit confirmation is key-driven only; full prompt/status UX is still minimal.
  - No file watching, no big-file tiers, no multi-buffer, no selection etc (per scope).
- Phase 2-c (terminal size / Screen for render height): render now uses App.screen (default 80x24 conservative; post-setup size() + Resize update); hardcoded 24 removed from App::render. Limitation: still full \x1b[2J clear, no real scrolling or viewport yet.
- Phase 2-d (minimal vertical cursor reveal/scroll_top): Screen::reveal_row(row) implemented using visible_height() as content area (bottom row reserved); scrolls up if row < scroll_top, scrolls down if row >= scroll_top + vh so row is at bottom of content. App::reveal_cursor called after char/Enter/backspace/delete/undo/redo/arrows and on resize (update+reveal+render). Render unchanged except scroll_top may advance. Tests: unit for reveal_row edges + app captured-output for down-past, post-reveal render emission, smaller-resize reveal. Limitation remains: no horizontal scroll, no smart viewport, no partial redraw, no big-file tiers yet.
- Phase 2-e (minimal horizontal cursor reveal/scroll_left): Screen now has scroll_left (init 0), visible_width() (uses width, 0->0, no reservation), reveal_col(col) using visible_width + saturating math (left edge, right edge so last visible, already-visible no-op, 0w safe, 1-col sane). App::reveal_cursor reads row+col and calls both reveal_row + reveal_col (all prior call sites). render_buffer takes extra start_col + width, does chars().skip(start_col).take(width) per line (0w emits no content but safe clear/pos), cursor screen_col = col - start_col + 1 (saturating). App passes screen.scroll_left/width. Tests added for reveal_col + app typing-past-width, render omits prefix after horiz reveal, narrow resize reveals cursor col. Default scroll_left=0 + sufficient width preserves all prior visible output and goldens. Limitations remain: scalar char columns only (no grapheme/wcwidth), no smart viewport, no partial redraw, no big-file tiers.
- Phase 2-f (viewport/render invariant hardening for zero-size/tiny terminals): Added Screen::clamp_scroll() (forces top/left=0 on vh/vw==0; nonzero leaves offsets alone). Wired clamp into handle_resize (after update) and reveal_cursor (before reveal). Added Screen invariant tests (zero h/w clamp respective scroll, nonzero preserve, repeated reveal still satisfies). Hardened render_buffer tests (h=0 no bottom pos/no panic; h=1 emits 0 content lines; w=0 no line content but safe clear/pos; start_col=0 preserves prior output). Added app tests (0x0 resize clamps+no panic; resize0->nonzero then type/move reveals; delete/bs after horiz scroll reduces scroll_left when cursor before viewport). No behavior change for normal sizes. Limitations remain: no grapheme/wcwidth, no smart viewport, no partial redraw, no buffer-aware max scroll, no big-file tiers.
- Phase 2-g (narrow cleanup): split App tests into src/app/tests.rs child module (use super::*; access preserved); app.rs reduced to 424 lines. No behavior change.
- Phase 2-h (buffer-aware viewport clamp): private App clamp_viewport_to_buffer() (uses buffer line_count + line for scalar limits); called in resize/reveal; vertical/horiz clamping after shrink/resize/move/delete/undo (scroll_top <= max or 0; scroll_left based on cursor line len). Added app tests for manual push+reveal/resize clamp, short buffer->0, horiz move-to-shorter + delete shorten, zero regression. Reveal preserved. Limitations remain: scalar char only (no wcwidth/grapheme), no predictive/smart viewport, clamp is reactive.
- Phase 2-i (narrow cleanup): split oversized src/app/tests.rs (>800 lines) into focused submodules under app::tests (viewport.rs, file_state.rs, editing.rs) while keeping tests.rs as small hub with pub(super) helpers. All tests descendants of app module (use super::super::*;); no behavior/API change, test names stable, paths now qualified under subs. app.rs 475 lines. rustfmt + full tests green.
- Phase 2-j (exact dirty via save-point token): conservative dirty=true on undo/redo replaced by exact token compare using Buffer::edit_history_position().
  - UndoStack now maintains current_id (monotonic, assigned on record; rewound on undo, restored on redo).
  - Save/open capture token; dirty = (current != saved). Undo back to saved clears dirty; redo away sets it; no-op undo/redo on clean stays clean; movement/render/resize untouched.
  - App helpers: refresh_dirty_from_buffer_history, mark_saved. App file_state tests cover all specified cases + quit guards preserved.
  - PieceTable tests for token advance/branch.
  - No full-buffer to_string compares on hot paths. SimpleBuffer: constant-0 stub only.
  - Remaining limitations: token is u64 internal (not exposed beyond minimal Buffer query); no multi-buffer or external-edit integration yet; still scalar char model.
- Phase 2-k (narrow cleanup): split src/buffer/tests.rs (was >800 lines post 2-j) into focused subs under buffer::tests (basic, storage_parity, edit_parity, undo_redo, model_parity, history_position). Hub now ~30 lines. All original test names preserved, paths updated (e.g. buffer::tests::undo_redo::*). No behavior change. app.rs stale comment updated. All tests green.
- Phase 2-l (narrow pass): on-disk file snapshot foundation (no watcher).
  - Added FileSnapshot (Present{len, mtime} / Absent) + ExternalFileStatus (NoPath/Unchanged/Modified/Deleted/Unknown) in file/io using only std::fs::metadata.
  - capture_file_snapshot: missing -> explicit Absent (no error); other IO errs propagate or Unknown.
  - Stored in FileState.disk_snapshot on App::new (existing=Present, missing=Absent, none=None) and on successful Ctrl+S.
  - Save failure leaves snapshot unchanged.
  - New App::external_file_status() (delegates to pure file_state helper): detection only; never mutates state.
  - Required tests in file/io and app::tests::file_state all pass.
  - Limitation (documented): same-length + same-mtime external overwrite is not detected (no content hash or full read this pass).
  - No notify, no background, no event loop polling, no reload UI, no conflict UX. Future 2+ will build watcher + handling on this.
- Phase 2-n (narrow pass): save-conflict guard (first+second Ctrl+S) using existing snapshot + ExternalFileStatus.
  - Added pending_save_conflict on App; factored do_atomic_save; guard in Ctrl+S before write.
  - First S on Modified/Deleted/Unknown for path: refuse, keep dirty, set message, record pending.
  - Second S with same still-conflicting status: force (updates saved token + disk snapshot).
  - Unchanged or NoPath (untitled): normal save, no check.
  - Clears on content edits/success; movement untouched.
  - If status kind changes between presses: update pending/msg, do not force.
  - Required app::tests::file_state cases added (external mod/delete, force, Absent->Present, change-between, edit-clears, untitled).
  - Limitations (addressed in 2-p): no watcher/reload UI yet; (prior) same-variant drift forced; Unknown primarily at io level; metadata-only (len/mtime). All Phase 2-l/m tests remain green.
- Phase 2-o (narrow cleanup): extracted save logic to src/app/save.rs (mod.rs <500); split file_state tests into dirty/snapshot/save_conflict submodules under tests/file_state/. All original test names preserved. No behavior change. See Phase 2-n for the hardening TODO (store conflict token not just status variant).
- Phase 2-p (narrow hardening): save-conflict now binds pending to observed (path + status + live FileSnapshot) via observe_external_file, not status variant alone.
  - Modified force only on identical live snapshot; Deleted/Unknown match by kind; drift between first/second S updates token and refuses again.
  - Added 6 targeted save_conflict tests for drift, Absent-appear, cross-status, and regression same-snapshot force.
  - Prior 2-n "same-variant drift treated as same" limitation addressed (no watcher/reload still; metadata-only unchanged).
  - Dead do_atomic_save forwarder removed while touching app/mod.rs. All prior tests green.

---

**Additional archived content (moved from TODO.md after 2-at):**

- Phase 2-r through 2-v: manual Ctrl+R status/reload path and metadata observation were hardened. Ctrl+R first reports/arms Modified or Deleted, second confirms reload/clear if the captured snapshot still matches; read failures do not lie; newline/content edits clear pending reload; observe_external_file uses one captured result without fs::metadata fallback on hard errors. No watcher/background/polling/hash/full-read/new deps were added in this slice.
- Phase 2-w through 2-ae: file watching became a Plain-safe, capability-gated, App-owned best-effort watcher. The watcher is constructed only when `file_watch` is true and a watchable parent exists, signals are hints only, and runtime checks at most once per loop through the app/watch helper. Modified/Deleted only arm the existing manual Ctrl+R confirmation path; Unchanged/NoPath suppress self-save noise or clear stale pending reload; no auto-reload and no content read from watcher signals. Deterministic queued seams and acceptance tests cover arming plus manual follow-up; live OS notify smoke stays ignored/manual.
- Phase 2-af through 2-aj: Phase 2B big-file foundation landed. File size metadata/tiering, pre-read guardrails, Large/Huge warnings, Extreme refusal, split perf harness, ignored 10/100 MiB + sparse manual smokes, recorded manual baselines, open-size guardrail extraction, and the initial persistent large-file status marker were added. Large/Huge still fully read and materialize; no thresholds or lazy storage were claimed.
- Phase 2-ak through 2-ar: Phase 2B open/materialization hygiene continued. Advisory budgets and hotspot inventory were documented; status size was clarified as disk metadata; key handling and render paths were cleaned up; open metadata capture was reduced to one snapshot; LF-only normalization, owned open/reload construction, std newline search in LineIndex, explicit OpenContentPlan, and centralized `file::io::read_to_string` reduced full-materialization overhead without changing the core limitation. Manual samples showed read_to_string as the main no-newline 100 MiB subphase after these narrow optimizations; full materialization remained.
- Phase 2-as: added ignored line-heavy 10/100 MiB manual open smokes plus a tiny default exact-size generator smoke. Docs recorded samples showing LineIndex/PieceTable cost reappears for newline-rich 100 MiB content. No default large tests, thresholds, new deps, or storage-policy changes.
- Phase 2-at: generated-file helpers switched from tiny repeated writes to buffered repeating-pattern writes, making manual fixture setup much cheaper while preserving exact sizes and stable labels. Docs record that generation timing is harness setup cost and must not be treated as editor performance.
- Phase 2-au through 2-ax: behavior-preserving storage-policy seams landed. App open content-plan buffer construction moved into app/open.rs, initial LineIndex construction now has a direct text path, and PieceTable original-source storage is wrapped in OriginalBacking::Owned without exposing borrowed slices to callers, so future lazy/mmap storage has a clearer boundary below Piece ranges. No file I/O moved into buffer, no thresholds were added, and Large/Huge present files still full-read/full-materialize.
- Phase 2-ay: Huge present files now open through an explicit read-only LargeFileBuffer instead of editable PieceTable materialization. The buffer validates UTF-8, scans line starts once, serves visible render windows with bounded positioned reads, and reports read-only edit/save attempts at the App layer. Confirmed Modified reload reapplies the same Large/Huge/Extreme policy instead of forcing Huge through the old full-read reload path. Small/Large editable opens still use PieceTable. Extreme still refuses before content read. This is a limited storage path, not mmap/rope/editable lazy loading, and external in-place mutation while open still needs a stronger snapshot story.
- Phase 2-az: LargeFileBuffer scan now records per-line ASCII flags. Visible windows on ASCII Huge lines use direct byte-offset reads, while non-ASCII windows keep scalar-safe scanning. The ignored sparse 1 GiB smoke now includes a far-horizontal-window render sample. This targets horizontal navigation/render on long ASCII lines without changing the storage policy or adding timing gates.
- Phase 2-ba: LargeFileBuffer scan now records sparse per-line char-column to byte-offset checkpoints. Non-ASCII Huge visible windows seek to the nearest prior checkpoint and scan forward from there, preserving scalar column semantics while avoiding start-of-line rereads for far horizontal windows. Added focused checkpoint tests and an ignored 100 MiB dense non-ASCII far-window manual smoke. No editable Huge semantics, mmap, immutable snapshot, or timing gate.
- Phase 2-bb: LargeFileBuffer now scans and reads through the same file descriptor, records fd len/mtime at open, and fails closed if descriptor metadata changes before ranged reads. This removes the scan/read reopen race and keeps already-open Huge buffers on the original descriptor after atomic path replacement. It is still metadata-only and does not make Huge files editable or provide a full immutable same-inode snapshot.
- Phase 2-bc: App::run panic-hook handling moved behind terminal::PanicRestoreGuard. The hook still restores terminal state during panic unwinds and chains to the previous hook, but normal exit now restores the previous hook instead of leaking a global hook. Unit coverage remains no-real-PTY but now verifies hook restore/chaining behavior.
- Phase 2-bd: A real root PTY integration smoke landed with `portable-pty` as a dev-dependency only. It drives the compiled `catomic` binary through save, undo, save, and clean quit over a pseudo-terminal, then asserts the saved file content. This turns the prior save/undo PTY stub into default executable coverage while keeping watcher/live notify and broader terminal flows out of scope.
- Phase 2-be: file-backed visible-window reads gained a fallible Buffer seam used by terminal rendering. LargeFileBuffer now returns descriptor/read errors through that seam, and render propagates them instead of silently displaying empty content. Existing infallible query compatibility remains for non-render callers; broader Huge query/edit error semantics are still open.
- Phase 2-bf: the real PTY integration harness gained an external-edit confirmation/reload smoke. It edits an open file from the test process, synchronizes with either watcher-armed or manual-first-press state, requires confirmed Ctrl+R reload content to render, then cleanly quits. Focused repeated runs guard against the watcher timing making this default acceptance test flaky.
