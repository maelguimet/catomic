# Architecture

This document describes Catomic's current, durable system boundaries. Historical
delivery phases and their verification records live under [`docs/progress/`](progress/)
and are not implementation requirements.

## Interaction flow

```text
terminal event -> normalized input -> semantic command -> state change -> render
```

Rendering reads editor state and must not mutate it. Terminal-specific input is
normalized at the boundary; editor commands must not reach into buffer internals.

## Construction

Catomic has one editor mode:

- Editor construction, typing, and rendering perform no repository scans,
  background indexing, configured processes, credential reads, or built-in
  network requests. Configured `on_open` and `on_save` hooks run only at those
  lifecycle events.
- Linting and trusted external commands are direct editor actions. Both remain
  bounded, cancellable, and absent until invoked.

There is no persistent workspace, project session, or global capability bouncer.
File watching, Markdown presentation, syntax, and completion remain local to
the active editor/file state.

Catomic has no built-in model provider, AI prompt/runtime, or repository-aware
assistant. Legacy AI configuration shapes are accepted only as inert upgrade
compatibility data and construct no service or action. This product boundary is
recorded in
[decision 0015](decisions/0015-no-built-in-ai-runtime.md).

The explicit `catomic update` command is outside the interactive editor loop and
may contact the documented GitHub source. Trusted configured linters, commands,
and hooks may also access the network because they are user-configured code;
Catomic bounds and reaps their processes but does not sandbox their effects.

## Ownership boundaries

- `src/main.rs` wires argument handling, application construction, and execution.
  `src/cli.rs` and `src/update/` own explicit non-editor command workflows.
- `src/app/` owns application state, the event-loop orchestration, semantic input
  routing, temporary surfaces, and workflows that coordinate subsystems.
- `src/terminal/` owns terminal sessions, raw-mode and protocol setup, signals,
  ANSI presentation, screen output, and terminal capability quirks. It does not
  implement editor commands.
- `src/buffer/` owns text storage, queries, mutations, and edit history. It does
  not perform terminal, filesystem, repository, or network work.
- `src/editor/` owns pure editing concepts such as document coordinates,
  selection, search, completion, syntax classification, and Markdown preview.
- `src/file/` owns file identity, loading, atomic saving, text formats, size
  policy, external-change watching, and recovery storage.
- `src/config/` owns typed configuration, validation, defaults, and keybinding
  translation. Loading configuration must not construct the services it names.
- `src/external/` owns bounded child-process execution primitives. User-facing
  command policy, confirmation, preview, and apply state stay in `src/app/`.
- `src/tests/` contains crate-internal golden, performance, and PTY helpers;
  top-level `tests/` exercises the compiled binary.

Cross-boundary work should keep policy with its owner. Input routes semantic
actions rather than mutating storage directly; filesystem code reports outcomes
rather than choosing UI state; workers return bounded results rather than owning
`App`; and rendering consumes immutable state.

## State and lifecycle rules

Prefer explicit state transitions over hidden side effects. Temporary surfaces
such as help, configuration, previews, prompts, and dialogs must define how the
previous editor context is restored. Background tasks must have bounded inputs,
outputs, and lifetimes, and dropping their owner must cancel or reap their work.

Hot typing and rendering paths must not acquire full-buffer clones, full-file
scans, blocking subprocesses, repository work, or network access. Suspected
performance problems should be measured before adding caches or concurrency.

## Source documentation

Module documentation should record real ownership, invariants, and non-obvious
safety constraints when they help a reader. There is no mandatory header
template. Historical phase labels, completion ledgers, and comments that merely
narrate the code do not belong in active source files.

Accepted design decisions under [`docs/decisions/`](decisions/) provide detail
for boundaries whose tradeoffs need a longer record. Engineering workflow,
testing, naming, and review rules remain in [`AGENTS.md`](../AGENTS.md).
