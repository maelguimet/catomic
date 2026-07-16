# Phase 4 Acceptance Record

Last verified: 2026-07-16, post 4-c.

This is the concise exit record for Phase 4. Detailed implementation history is
in `progress/phase-4-progress.md`; measurements are also retained in
`performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| Extension syntax | `.md`, Rust, Python, and JSON select pure scalar-indexed lexical rules. Visible headings, Markdown markers/code, language keywords, strings, comments, and numbers compose with search/selection reverse video. |
| Bounded rendering | Syntax receives only lines returned by the existing visible-window Buffer query. Ordinary render does not parse, cache, clone, or scan the full document and constructs no worker. |
| Markdown preview | `F6` explicitly parses the active Markdown buffer or active large-file page with `pulldown-cmark` into a separate read-only PieceTable. It renders headings, lists, blockquotes, inline/code blocks, tasks, and tables without modifying source history. `F6` or Escape restores the source viewport. |
| View indicators | `F7` toggles a fixed line-number gutter and `F8` toggles one-cell space/tab indicators. Cursor reveal, resize, horizontal scroll, and mouse mapping account for gutter width; settings remain per buffer. |
| Read-only safety | Ordinary key input and bracketed paste cannot edit preview source; mouse selection is ignored there. External reload cancels stale preview state before replacing source. |
| Golden preview | `golden_markdown_preview_document` compares the complete rendered preview string for heading, list, inline-code, and blockquote input while asserting the source fixture remains byte-identical. |
| Real terminal flow | The default PTY smoke sends F6/F7/F8 to the release-shaped binary, requires rendered/read-only/toggle messages, verifies the source file is unchanged, and requires mouse, bracketed-paste, and alternate-screen teardown. |
| Plain startup discipline | `pulldown-cmark` is invoked only by explicit F6. Startup, typing, and ordinary viewport rendering do not construct a parser or add Project/LLM/network services. Dependency scope and removal are recorded in decision 0005. |

## Performance target and result

The Phase 4 local release targets are: build a deliberately requested 10 MiB
Markdown preview in under 150 ms; redraw 1,000 fully styled 80x24 viewports from
that 10 MiB buffer in under 100 ms; keep the complete measurement process below
128 MiB peak RSS. These are acceptance budgets, not default timing gates.

`manual_phase4_10mib_markdown_reports_samples` measured **92 ms** for preview
construction and **15-18 ms** for 1,000 styled viewport renders. The timed run
reported **125,424 KiB** peak RSS. Preview construction is one explicit action;
each subsequent render requests only the final 23 logical rows.

## Manual UX checklist

A live 80x24 PTY session against the release binary and a Markdown fixture
verified:

- edit mode colored the heading, quote/list markers, fence delimiters, and
  inline code without obscuring source punctuation;
- F6 produced readable heading, list, blockquote, inline-code, and fenced-code
  output, and an attempted `x` edit produced the read-only guard;
- F7 pinned aligned line numbers and shifted the cursor/content area correctly;
- F8 made spaces visible without changing logical cursor coordinates;
- arrow navigation moved inside preview, Escape restored the source view, and
  clean Ctrl+Q emitted all inverse terminal-mode sequences;
- the source fixture remained unchanged.

## Verification commands

- `cargo test --quiet`: 364 passed, 11 intentional manual tests ignored.
- `cargo test --quiet --test pty_smoke`: 5 passed.
- `cargo test --release --quiet manual_phase4_10mib_markdown_reports_samples -- --ignored --nocapture`: 1 passed; 92 ms preview and 15-18 ms for 1,000 renders.
- `/usr/bin/time` around that release test: 125,424 KiB peak RSS.
- `cargo build --release --quiet`: passed.
- `cargo fmt -- --check` and `git diff --check`: passed for the acceptance slice.

## Result

Phase 4 acceptance is complete. Catomic now has useful, bounded light syntax,
an explicit terminal-native Markdown preview, and optional view indicators
without adding idle work or Project-mode cost to Plain editing.
