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

- Startup and ordinary editing perform no repository scans, background
  indexing, configured commands or hooks, model probing, credential reads, or
  network requests.
- Linting is a direct editor action. Repository-aware model context is detected
  and prepared afresh for each explicit request. Both remain bounded,
  cancellable, and absent until invoked.

There is no persistent workspace, project session, or global capability bouncer.
File watching, Markdown presentation, syntax, and completion remain local to
the active editor/file state.

Model-backed actions have an additional explicit confirmation boundary. Clients
and command processes are transient, model output is untrusted, and edits remain
preview-first. The complete contract is in [`llm-rules.md`](llm-rules.md).

## Ownership boundaries

- `src/main.rs` wires argument handling, application construction, and execution.
  `src/cli.rs` and `src/update/` own explicit non-editor command workflows.
- `src/app/` owns application state, the event-loop orchestration, semantic input
  routing, temporary surfaces, and workflows that coordinate subsystems.
- `src/terminal/` owns terminal sessions, raw-mode and protocol setup, signals,
  ANSI presentation, screen output, and terminal capability quirks. It does not
  implement editor commands.
- `src/buffer/` owns text storage, queries, mutations, and edit history. It does
  not perform terminal, filesystem, repository, or model work.
- `src/editor/` owns pure editing concepts such as document coordinates,
  selection, search, completion, syntax classification, and Markdown preview.
- `src/file/` owns file identity, loading, atomic saving, text formats, size
  policy, external-change watching, and recovery storage.
- `src/config/` owns typed configuration, validation, defaults, and keybinding
  translation. Loading configuration must not construct the services it names.
- `src/llm/` owns bounded context, backend adapters, request workers, brokered
  request-local repository reads, proposal parsing, and model-specific safety
  limits. It does not own application state or silently write files.
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
