# Phase 3 Acceptance Record

Last verified: 2026-07-16, post 3-e.

This is the concise exit record for Phase 3. Detailed implementation history is
in `progress/phase-3-progress.md`; measurements are also retained in
`performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| Incremental search | Ctrl+F moves and reverse-highlights while the query changes. Enter/Down and Up navigate with wraparound; Escape clears prompt, worker, and highlight state. Unicode positions use scalar document columns. |
| Oversized-file search | Explicit cancellable workers scan the stable descriptor in bounded chunks, include retained unsaved page overlays and seam-forming edits, and navigate forward/backward across pages. No worker exists before invocation. |
| Goto and command surface | Ctrl+G accepts a 1-based line and clamps ordinary or paged files. Ctrl+Shift+P supports `goto N`, `save`/`write`, and `quit`/`q` through existing safe paths. Both prompts cancel without editing. |
| Selection and replacement | Shift+arrows and Ctrl+A create half-open multiline scalar-coordinate selections. Typing, newline, delete, cut, internal paste, and bracketed paste replace the range through one Buffer transaction. |
| Clipboard | Ctrl+C/X/V always use a process-local clipboard. Bounded copies also emit OSC 52; oversized selections remain available internally without emitting an unbounded terminal sequence. |
| Mouse | Captured left clicks map through both viewport offsets, drags select, status-row clicks are ignored, and double-click expands Unicode word or punctuation runs. Teardown disables mouse capture on normal exit and panic cleanup. |
| Golden search/replacement | `golden_search_selection_replace_and_save` finds the first exact match, replaces only that range, atomically saves, and compares the complete output file with `alpha cat omega\ntarget stays`. Phase 3 does not add a separate replace prompt. |
| Real terminal Ctrl+F | The default PTY smoke sends Ctrl+F and a query separately, requires the live reverse-video match before Enter, then exits cleanly. |
| Plain startup discipline | Search workers are explicit and cancellable; selection, prompts, clipboard, and mouse handling construct no Project, LLM, indexer, or network machinery. |

## Performance target and result

The Phase 3 reference target is a worst-position forward search through a
10 MiB line-heavy in-memory PieceTable in under 100 ms, with the full release
test process below 64 MiB peak RSS on the acceptance machine. The target is a
local release acceptance budget, not a default-test timing gate.

`manual_search_10mib_line_heavy_buffer_reports_sample` places the only query at
EOF so the complete buffer is visited. On this machine it completed in **8 ms**;
`/usr/bin/time` reported **32,984 KiB** peak RSS for the release test process.
The ignored fixture retains correctness assertions and a stable sample label for
future remeasurement.

## Manual UX checklist

A live 80x24 PTY session against the release binary and a README copy verified:

- Ctrl+F progressively highlighted `C`, `Ca`, through `Catomic`; Enter moved to
  the next visible occurrence and Escape removed the search state.
- Ctrl+G `20` reported `Moved to line 20.` and placed the cursor on that row.
- two Shift+Right events grew the visible selection by one scalar each.
- SGR mouse click/drag visibly selected text, and a double click selected the
  complete word `Obvious`.
- clean Ctrl+Q emitted the inverse mouse, bracketed-paste, and alternate-screen
  sequences and left the copied fixture unchanged.

Terminal-emulator interception of Ctrl+Shift+C/V remains explicitly documented;
the always-available internal clipboard is not dependent on those bindings.

## Verification commands

- `cargo test --quiet`: 348 passed, 10 intentional manual tests ignored.
- `cargo test --quiet --test pty_smoke`: 4 passed.
- `cargo test --release --quiet manual_search_10mib_line_heavy_buffer_reports_sample -- --ignored --nocapture`: 1 passed; 8 ms search sample.
- `/usr/bin/time` around that release test: 32,984 KiB peak RSS.
- `cargo build --release`: passed.
- `cargo fmt -- --check` and `git diff --check`: passed for the acceptance slice.

## Result

Phase 3 acceptance is complete. Search, goto, the minimal command surface,
selection replacement, clipboard paths, bracketed paste, and mouse interaction
are implemented and verified without adding idle Plain-mode services.
