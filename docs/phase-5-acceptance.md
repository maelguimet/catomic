# Phase 5 Acceptance Record

Last verified: 2026-07-16, post 5-e.

This is the concise exit record for Phase 5. Detailed implementation history is
in `progress/phase-5-progress.md`; measurements are also retained in
`performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| Capability bouncer | Plain startup constructs only its optional local-completion state. `:project`/`:code` creates the Project session explicitly; `:plain`/`:text` drops the session and all owned workers/results. |
| Local completion | Candidate collection is capped at 257 rows, 1,024 columns per row, a 512-column prefix, and 16 results. It uses only current-buffer text, stays process/index-free, and accepts as one undoable range replacement. |
| Linter execution | Lazy `[linters]` configuration maps normalized extensions to commands containing `{file}`. `:lint` requires Project mode and a saved file, runs asynchronously, supports cancellation, and caps each output stream at 1 MiB. |
| Diagnostics | Common path/line/column/severity output is parsed into a read-only list. `:dnext`/`:dprev` jump within the active file or to an already-discovered cross-file target; missing targets fail visibly. |
| Project discovery | `:files` starts the only scan. The cancellable worker caps results at 4,096 files, 65,536 entries, and depth 64; skips symlinks, `.git`, `node_modules`, and `target`; sorts complete results; and reports truncation/errors. |
| File picker | The transient read-only picker supports arrows, Page Up/Down, Home/End, Enter open/reuse, and Escape close/cancel without mutating a buffer. |
| Project path completion | Path-like prefixes use only the cached result of the most recent explicit `:files`. Completion never starts discovery and tells the user when no cache exists. |
| Exact golden | The Phase 5 golden checks exact sorted path candidates, exact accepted document text, and exact undo restoration. |
| Real terminal flow | The PTY smoke enters Project mode through CSI-u input, runs discovery, closes the picker, accepts cached path completion, saves exact text, quits, and requires terminal teardown. |
| Plain discipline | Plain mode performs no project scan, config load, subprocess launch, index construction, LSP work, or network work. Path-like text still receives only local current-buffer candidates there. |

## Performance target and result

The Phase 5 local release targets are: discover a deliberately requested
4,096-file tree in under 50 ms; perform 100 cached path-completion requests in
under 10 ms; keep the complete warm measurement process below 64 MiB peak RSS.
These are acceptance budgets, not default timing gates.

`manual_phase5_4096_file_project_reports_samples` measured **2 ms** for bounded
discovery and **under 1 ms** for all 100 cached completion requests. The warm
release invocation reported **33,480 KiB** peak RSS. Fixture creation occurs
before either timed sample.

## Terminal UX checklist

The real 80x24 PTY flow verified:

- Plain startup displayed the source without constructing Project state;
- `:project` enabled Project mode and `:files` reported exactly two files;
- the picker displayed the discovered path and Escape restored the source;
- `Ctrl+Space` offered the cached `src/main.rs` candidate for `src/ma`;
- Enter accepted the candidate, Ctrl+S wrote exact source text, and Ctrl+Q
  emitted the terminal teardown sequences.

Unit integration coverage additionally runs an actual shell linter, polls its
asynchronous result, checks parsed diagnostics and navigation, and cancels a
long-running linter when Project mode is left.

## Verification commands

- `cargo test --quiet`: 407 passed, 12 intentional manual tests ignored.
- `cargo test --quiet --test pty_smoke`: 6 passed.
- `cargo test --release --quiet manual_phase5_4096_file_project_reports_samples -- --ignored --nocapture`: 1 passed; 2 ms discovery and under 1 ms for 100 completions.
- `/usr/bin/time` around the warm release test: 33,480 KiB peak RSS.
- `cargo build --release --quiet`: passed.
- `cargo fmt -- --check` and `git diff --check`: passed for the acceptance slice.

## Result

Phase 5 acceptance is complete. Catomic now has bounded current-buffer
completion in Plain mode and explicit, lazy linter/discovery/path tooling in
Project mode without adding idle Project cost to ordinary editing.
