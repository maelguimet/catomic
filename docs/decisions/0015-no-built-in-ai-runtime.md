# Decision 0015: No Built-In AI Runtime

Date: 2026-07-25

Status: accepted

## Context

Catomic's core promise is a small, modeless terminal editor that feels closer
to a capable Nano than to a persistent IDE. Model-backed editing accumulated a
second product inside that editor: provider and command adapters, prompt
construction, context collection, repository inspection, model selection,
discovery, asynchronous request state, proposal parsing, and dedicated render
surfaces. Even when lazy and bounded, that machinery increased the executable,
configuration, maintenance, and security surface.

Non-AI editor features are independently useful and remain in scope. These
include syntax highlighting, Markdown preview, local completion and emoji
insertion, lint diagnostics, trusted external commands and lifecycle hooks,
multiple buffers, large-file paging, recovery, file watching, automatic reload,
and external-change marks.

## Decision

Catomic has no built-in AI or model integration. Remove:

- current-file and repository-aware model commands;
- inline instruction/clanker workflows and model-change marks;
- model picker, discovery, session selection, and provider presets;
- HTTP and command model adapters, credential lookup, transcripts, prompts,
  context brokers, Git snapshots, proposal parsers, and request tasks; and
- AI-specific actions, shortcuts, help, configuration generation, and render
  state.

Generic editor mechanisms do not become AI-specific merely because an earlier
feature used them. Keep the F2 command prompt, generic confirmation and preview
surfaces, Markdown preview, recovery preview, external-command preview/apply,
diff styling used by retained features, and bounded child-process primitives.

## Network and process boundary

Startup and interactive editing make no Catomic-owned network request. Catomic
does not contain an editor path that sends buffer or repository content to a
model service.

This does not make the complete program network-inert:

- the explicit `catomic update` command contacts the documented GitHub source
  to check or apply updates; and
- trusted configured linters, commands, and hooks may access the network or
  have other external effects because they are user-configured shell code.

Those exceptions do not construct a model runtime or permit hidden network work
in startup, typing, rendering, completion, linting, file watching, or preview
paths.

## Upgrade compatibility

The updater validates an installed configuration with the candidate binary
before replacement. To avoid trapping users on an older release, strict parsing
continues to recognize retired AI configuration shapes as inert data:

- `[llm]`, its legacy fields, `[[llm.backends]]`, and `[llm.inline]`;
- `[languages.EXT.llm.inline]`;
- `hooks.before_llm`;
- backend `models` and `discovery`;
- the `run-clanker`, `clear-clanker-changes`, `select-model`, `picker-accept`,
  and `picker-cancel` action names; and
- `theme.colors.llm_changed`.

Recognized containers retain their type and unknown-key checks. Their values do
not construct providers, read credentials, start commands, create shortcuts,
prepare requests, or affect presentation. New configuration omits them.
The retired prompt spellings `meow`, `bigmeow`, `gitmeow`, `megameow`,
`inline-meow`, `model`, `models`, and `select-model` remain unknown commands;
they are not broadened into action compatibility aliases.

This compatibility exception is a parsing contract, not a dormant feature
flag. Reintroducing runtime behavior behind these fields requires a new product
decision.

## Consequences

Catomic retains its non-AI editor and IDE conveniences while removing the
built-in model, prompt, networking, and repository-AI implementation. The
remaining generic command surface lets users choose their own trusted
automation without Catomic owning a provider protocol or agent workflow.

Decisions 0008 and 0013 are superseded. The AI-specific coordination described
in decision 0014 is historical; its generic App/input/render ownership rules
remain current.
