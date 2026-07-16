# Phase 2 Acceptance Record

Last verified: 2026-07-16, post 2-ca.

This is the concise exit record for Phase 2. Detailed implementation history is
in `progress/phase-2-progress.md`; measurement history is in `performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| External edits do not silently overwrite work | Clean buffers auto-reload by default after a fresh metadata observation; dirty buffers preserve local work and require matching Ctrl+R confirmation. Auto reload can be disabled. Save conflicts include Unix identity/change time and same-length replacement coverage. |
| Atomic save and dirty tracking | Save streams to a same-directory temp, syncs, renames, preserves existing Unix mode bits, and tracks the exact saved history position. Failed writes preserve the target. |
| Real terminal flows | Four default PTY tests cover save/undo/save, external reload confirmation, Ctrl+F, and switching/editing/saving the active file in a two-buffer session. |
| Configurable oversized-file pages | `[big_files] page_lines` defaults to 20,000 and is loaded from XDG/HOME config. Ctrl+PageUp/PageDown changes pages and status reports page/byte range. |
| Editable files at every supported size | Huge/Extreme files use file-backed PieceTable pages. Edits are retained across pages, global undo/redo follows transaction order, and Ctrl+S atomically streams edited pages plus untouched descriptor ranges as one document. Byte size alone never selects read-only or refusal behavior. |
| Search across all pages | Explicit cancellable Ctrl+F streams the stable descriptor plus unsaved edited-page overlays in bounded chunks, preserves matches across chunk and edited-page boundaries, and jumps to the page containing the match. |
| Large-file open and navigation | Current manual suites pass for editable 10 MiB, paged 100 MiB ASCII/Unicode/line-heavy, sparse 1 GiB, and sparse >1 GiB files. Exact timings are recorded in `performance.md`. |
| Rendering | Rendering is viewport-only and clears/repaints rows without a terminal-wide full-screen clear. |
| Multiple buffers | Every positional CLI path opens in order; Alt+PageUp/PageDown switches a state-preserving ring; dirty inactive buffers participate in quit protection. |
| Status | Persistent status reports mode, path, saved/modified state, UTF-8, size/tier, page range, and active buffer position when applicable. |
| Plain startup discipline | Paging/config/watcher behavior is local and Plain-safe; it constructs no Project, LLM, background search, or network machinery. |

## Verification commands

- `cargo test`: 313 passed, 9 intentional manual tests ignored.
- `cargo test --test pty_smoke`: 4 passed.
- `cargo test tests::perf::manual:: -- --ignored --nocapture`: 5 passed.
- `cargo test tests::perf::manual_line:: -- --ignored --nocapture`: 2 passed.
- Live watcher smoke (`live_smoke_watcher_sees_external_change_and_auto_reloads` with
  `--ignored --nocapture`): 1 passed against the live notify backend.
- `cargo build --release`: passed.
- `cargo fmt -- --check` and `git diff --check`: pass for committed changes.

## Result

Phase 2 acceptance is complete. Oversized pages are editable, whole-document
save and cross-page history are verified, clean external changes auto-reload by
default, and dirty buffers retain explicit conflict/reload protection.
