# Phase 2 Acceptance Record

Last verified: 2026-07-16, post 2-bw.

This is the concise exit record for Phase 2. Detailed implementation history is
in `progress/phase-2-progress.md`; measurement history is in `performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| External edits do not silently overwrite work | Metadata snapshots include Unix identity/change time; Ctrl+S and Ctrl+R require a matching second confirmation when disk state conflicts. Unit coverage includes same-length replacement with restored mtime. |
| Atomic save and dirty tracking | Save streams to a same-directory temp, syncs, renames, preserves existing Unix mode bits, and tracks the exact saved history position. Failed writes preserve the target. |
| Real terminal flows | Four default PTY tests cover save/undo/save, external reload confirmation, Ctrl+F, and switching/editing/saving the active file in a two-buffer session. |
| Configurable oversized-file pages | `[big_files] page_lines` defaults to 20,000 and is loaded from XDG/HOME config. Ctrl+PageUp/PageDown changes pages and status reports page/byte range. |
| Search across all pages | Explicit cancellable Ctrl+F streams the stable descriptor in bounded chunks and jumps to the page containing the match. |
| Large-file open and navigation | Current manual suites pass for editable 10 MiB, paged 100 MiB ASCII/Unicode/line-heavy, sparse 1 GiB, and sparse >1 GiB files. Exact timings are recorded in `performance.md`. |
| Rendering | Rendering is viewport-only and clears/repaints rows without a terminal-wide full-screen clear. |
| Multiple buffers | Every positional CLI path opens in order; Alt+PageUp/PageDown switches a state-preserving ring; dirty inactive buffers participate in quit protection. |
| Status | Persistent status reports mode, path, saved/modified state, UTF-8, size/tier, page range, and active buffer position when applicable. |
| Plain startup discipline | Paging/config/watcher behavior is local and Plain-safe; it constructs no Project, LLM, background search, or network machinery. |

## Verification commands

- `cargo test`: 301 passed, 9 intentional manual tests ignored.
- `cargo test --test pty_smoke`: 4 passed.
- `cargo test tests::perf::manual:: -- --ignored --nocapture`: 5 passed.
- `cargo test tests::perf::manual_line:: -- --ignored --nocapture`: 2 passed.
- Live watcher smoke (`live_smoke_watcher_sees_external_change_and_arms` with
  `--ignored --nocapture`): 1 passed against the live notify backend.
- `cargo build --release`: passed; produced a 794 KiB x86-64 Linux ELF binary.
- `cargo fmt -- --check` and `git diff --check`: pass for committed changes.

## Spec reconciliation still required

Two original Phase 2 bullets conflict with the later conservative decisions and
the current implementation:

1. The original watcher text says a clean external change auto-reloads. Current
   behavior always requires explicit Ctrl+R confirmation, avoiding an
   asynchronous replacement of visible editor state.
2. The original 100 MiB/1 GiB text requires local edits. The accepted paging
   decision opens files above 100 MiB read-only until cross-page edit/save and
   immutable-snapshot semantics are defined.

Phase 2 should be declared complete only after the original bullets are updated
to the accepted conservative behavior, or after the implementation is expanded
to meet the older auto-reload and oversized-editing requirements.
