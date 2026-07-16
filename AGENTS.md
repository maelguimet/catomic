# AGENTS.md — Catomic Rules

Catomic is a Linux-first terminal text editor: fast, modeless, boring-core-first.

Read this before changing code.

See also: [TODO.md](./TODO.md), [README.md](./README.md), and `docs/` (architecture, performance, LLM rules, decisions).

## Prime Directive

Build the cursor before the cathedral.

Current phase: Phase 5 complete; Phase 6 implementation work.

Phases 0 through 5 are complete and have acceptance records in `docs/`:
- blessed terminal loop, Buffer abstraction, PieceTable, LineIndex, and undo/redo
- robust file handling, atomic save, external-change safety, and bounded large-file editing
- search, selection, multiple buffers, Markdown preview, and viewport-only syntax
- gated Plain/Project lifecycle, local completion, on-demand linting, diagnostics,
  bounded project discovery, and cached project-path completion

Allowed now:
* Phase 6 work defined in `TODO.md` and `docs/llm-rules.md`
* deterministic parser, patch, broker, preview, confirmation, and undo tests
* acceptance hardening and regression fixes for completed phases

Phase 6 feature work is allowed because Phase 5 acceptance tests are green.

Forbidden now:
* testing LLM features against a live model or endpoint
* silent network calls or silent writes
* UI feature expansion (beyond phase needs)

## Workflow

Before editing:

1. `git status --short`
2. inspect relevant files
3. make one small coherent change
4. run relevant tests
5. `git diff --check`
6. review `git diff`
7. commit when coherent

Use git aggressively. One logical step per commit.

No monster commits. No drive-by formatting. No overwriting uninspected changes.

## Architecture

Flow:

```txt
terminal event -> normalized input -> editor command -> state mutation -> render
```

Rules:

* `main.rs` stays tiny.
* `terminal/` owns crossterm, raw mode, ANSI, input decoding.
* `buffer/` owns text storage and edits only.
* `editor/` owns semantic commands, cursor, selection, search.
* `project/` is Project mode only and must not exist in Plain startup.
* `llm/` is explicit invocation only. No silent network. No silent writes.

Render reads state. It must not mutate state.

Input must not poke buffer internals directly.

## Plain vs Project

Plain/Text mode is default.

Plain mode must not construct:

* repo scanners
* linters
* LSP
* diagnostics
* background indexers
* repo-aware LLM machinery
* network clients unless explicitly invoked

Project/Code mode is opt-in, lazy, killable, and must never block typing.

## File and Function Limits

Prefer:

* files under 300 lines
* functions under 40 lines
* focused modules with one job

Over 500 lines needs justification.
Over 800 lines: split before adding more.
Over 1,000 lines: design smell unless generated/test fixture data.

No 10k-line files.

## Naming

Use boring, explicit names.

Good:

* `move_cursor_left`
* `insert_char_at_cursor`
* `apply_editor_command`
* `render_visible_rows`
* `snapshot_git_state`

Bad:

* `handle`
* `process`
* `do_it`
* `thing`
* `manager`
* `utils.rs`

## File Headers

Every non-trivial source file starts with:

```rust
//! Purpose: this file must ...
//! Owns: ...
//! Must not: ...
//! Invariants: ...
//! Phase: ...
```

Headers describe the contract, not a claim that the code is already correct.

## Dependencies

Phase 0 target: `crossterm` only.

Every new dependency must justify:

1. why std cannot do it,
2. which mode uses it,
3. whether it affects Plain startup,
4. how it is tested,
5. how to remove/disable it.

No Electron, webview, curses, or widget cathedral.

## Testing and Performance

Use TDD for:

* buffer
* piece table
* line index
* cursor mapping
* undo/redo
* patch application

No phase is complete until its acceptance tests pass.

Performance rule: measure, do not guess.

Never add full-file scans, full-buffer clones, background work, or network calls to hot paths.

## Scope Control

Keep clanker tasks small.

Good task:

* one module
* one narrow feature path
* one test harness
* one bug

Bad task:

* "implement Phase 0"
* "make editor work"
* "refactor architecture"

If a task needs more than ~8 files of context, split it.

If context is getting low, stop, test, commit or leave a clean diff, and write a handoff note.

## Done Means

Before saying done:

* tests pass
* formatting passes
* `git diff --check` passes
* diff reviewed
* no unrelated files changed
* files/functions stayed sane
* Plain mode did not gain Project cost
* commit made, or remaining uncommitted work stated clearly

Keep the goblin loop blessed.
