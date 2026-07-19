# Catomic Roadmap

Catomic is in open beta. Phases 0–8 of the original build plan are complete.
The full design and implementation record is preserved in
[docs/progress/roadmap-history.md](docs/progress/roadmap-history.md); completed
phase notes and acceptance records remain under [docs/](docs/).

This file contains only the current product contract, active work, dependencies,
and delivery rules. GitHub issues are the source of truth for individual
features and defects.

## Product contract

Catomic is both:

1. **a sane terminal text editor** — modeless, Nano-like, quick to open, obvious
   without configuration, and useful for ordinary text and code editing; and
2. **a clanker-native power tool** — repository context, model-backed edits, and
   other nuclear capabilities are available when deliberately invoked.

The second identity must not damage the first. A user who never configures or
invokes a model should get a complete, quiet editor without agent UI, background
repository work, surprise processes, or network activity.

Power stays hidden behind explicit commands, capability gates, scoped context,
destination confirmation, read-only proposal preview, and explicit apply. It is
not a permanently visible IDE mode and never becomes ambient automation.

## Non-negotiable invariants

- Plain mode constructs no Project services, performs no repository scan, starts
  no configured command or hook, and makes no network request.
- Entering Project mode is explicit. Individual Project services remain lazy and
  capability-gated.
- Opening a file, opening help, or opening a model picker must not construct an
  LLM client, read an API-key value, probe a provider, or start a model process.
- Model work begins only after explicit invocation and confirmation of the
  destination, model, and exact context scope. The sole automatic-call exception
  is disabled-by-default inline autocomplete after an equally explicit,
  bounded, active-buffer-only session confirmation.
- Model output is bounded and untrusted. Edits remain read-only until separately
  accepted, fail closed on drift or malformed ranges, apply as undoable buffer
  transactions, and never save automatically.
- Ordinary editing remains responsive and correct for Unicode graphemes,
  terminal-cell widths, multiple buffers, external file changes, and supported
  large-file tiers.
- Rendering never mutates editor state. Terminal, buffer, filesystem, Project,
  LLM, and configuration boundaries in
  [docs/architecture.md](docs/architecture.md) remain enforceable.
- Safety and performance regressions outrank feature volume.

## Current baseline

The beta already includes:

- modeless editing, selection, mouse input, search/replace, undo/redo, multiple
  buffers, familiar shortcuts, soft wrap, line numbers, and Markdown preview;
- grapheme-aware cursor movement and terminal layout;
- atomic saves, save-conflict protection, external-change detection, explicit
  reload/overwrite confirmation, and opt-in recovery;
- editable large-file pages with bounded retention and whole-document save/search;
- opt-in Project discovery, diagnostics, completion, commands, and hooks;
- explicit `meow`, `bigmeow`, `gitmeow`, and `megameow` workflows with
  bounded context, preview, drift checks, undo, and no automatic save;
- a one-key inline clanker workflow with selection/catblock/file precedence,
  bounded serial queueing, atomic cleanup, and semantic applied-change marks;
- opt-in inline continuation ghost text with bounded active-buffer-only context,
  session confirmation, cancellation, stale-response guards, and explicit apply;
- typed configuration and documented security, performance, and contribution
  boundaries.

Acceptance records under [docs/](docs/) describe the verified behavior. Do not
copy completed implementation journals back into this roadmap.

## Active work

### Beta correctness and visible polish

Correct advertised behavior before widening the platform surface:

- [#53](https://github.com/maelguimet/catomic/issues/53) — eliminate help-view
  redraw flicker.
- [#64](https://github.com/maelguimet/catomic/issues/64) — restore reliable
  click-to-position behavior.
- [#63](https://github.com/maelguimet/catomic/issues/63) — make the status and
  message row visually distinct.
- [#56](https://github.com/maelguimet/catomic/issues/56) — make redo bindings
  unambiguous.
- [#57](https://github.com/maelguimet/catomic/issues/57) — explain the model
  command variants concisely in built-in help.

### Configuration and customization foundation

- [#62](https://github.com/maelguimet/catomic/issues/62) is the parent design for
  config discovery/editing, complete action remapping, and semantic color
  schemes. Deliver it as reviewable vertical slices rather than one giant PR.
- [#58](https://github.com/maelguimet/catomic/issues/58) should use the resulting
  preference/state policy for persistent line-number defaults.
- Status, model-change, warning, and error colors must consume semantic theme
  roles instead of growing separate hard-coded ANSI paths.

### Clanker-native power

- [#67](https://github.com/maelguimet/catomic/issues/67) establishes model
  presets and the common HTTP/headless-command backend boundary.
- [#61](https://github.com/maelguimet/catomic/issues/61) should document the final
  selector/backend configuration after #67 stabilizes it.
- No model feature may weaken the Plain-mode, confirmation, bounded-context,
  preview, drift, undo, or no-auto-save guarantees.

### Editing and document UX

- [#54](https://github.com/maelguimet/catomic/issues/54) — improve Markdown source
  styling and preview rendering, especially tables.
- [#59](https://github.com/maelguimet/catomic/issues/59) — add explicit overwrite
  mode without contaminating ordinary insert, paste, prompt, or proposal paths.

### Distribution and additional platforms

- [#60](https://github.com/maelguimet/catomic/issues/60) — define safe,
  install-method-aware updates without overwriting user state.
- [#66](https://github.com/maelguimet/catomic/issues/66) — establish
  Android/Termux support and touch/soft-keyboard-accessible workflows.
- Mobile work follows the cursor/mouse, action-remapping, semantic-status, and
  narrow-layout foundations rather than duplicating them.

## Dependency order

1. Resolve beta correctness defects and small visible inconsistencies.
2. Establish the shared action registry, configuration, preferences, and
   semantic theme primitives from #62.
3. Establish the backend/preset abstraction in #67.
4. Finalize model help in #61 once configuration and picker behavior stop moving.
5. Reuse cursor, viewport, action, status, and theme primitives for mobile work.

Independent editor improvements may proceed in parallel when they do not cross
these foundations.

## Delivery rules

- One PR should deliver one coherent, reviewable behavior.
- Large issues are parent specifications, not permission for monolithic diffs.
  Split them into vertical slices while keeping their shared acceptance contract.
- Every behavior change needs proportional unit, integration, render, PTY, or
  manual evidence. Existing automated tests are baseline evidence, not a
  substitute for relevant manual terminal checks.
- Normal PR CI stays deterministic and reasonably fast.
- Ignored, environment-sensitive, or expensive acceptance checks run through
  the separate Acceptance workflow for release candidates and deliberate manual
  verification.
- Tests never contact a live public model endpoint or require paid credentials.
- Update user documentation with user-facing behavior. Put completed engineering
  history under `docs/progress/`, not in this roadmap.
- Preserve boring Rust, explicit ownership, bounded work, and minimal
  dependencies.

## Explicit non-goals

- Always-on agents, unconfirmed ambient model suggestions, or silent
  model/provider probing.
- Background repository indexing in Plain mode.
- Turning ordinary startup into an IDE dashboard or model configuration flow.
- Silent file writes, silent model edits, or automatic saves after model output.
- Hiding core editor functionality behind model setup.
- Replacing focused terminal workflows with feature volume for its own sake.

## Maintaining this roadmap

Keep this file short and current:

- add or update the relevant GitHub issue for detailed requirements;
- record only active priorities and dependency decisions here;
- move completed phase narratives and investigation logs to `docs/progress/`;
- remove completed items instead of appending permanent status diaries;
- update the product contract only when the intended user experience changes.
