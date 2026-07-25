# LLM Rules

No silent writes. No blind full-file replacement. No hidden network. No automatic workspace upload.

## Output Preference Order

1. unified diff/patch
2. marked region

Full-file replacement output is not accepted. A marked region uses only the
strict `{"catomic_replacement":"..."}` envelope.

Every patch or replacement LLM edit must be previewed, confirmed, undoable.

## Commands

- `:meow` — selection/block
- `:bigmeow` — current file
- `F3` / `run-clanker` / `inline-meow` — inline instruction with automatic
  `selection → catblocks → bounded full file` scope

Wide or multi-file patches are not accepted.

## Inline clanker

An inline instruction is control metadata, never an editable target. The
default is a trimmed line starting with `>>` followed by whitespace. Delimiters
must occupy their own trimmed lines. Configured markers are bounded and
non-ambiguous; nested, mismatched, overlapping, or unclosed context blocks fail
closed with line numbers. Existing `>>> catomic ... <<<` instruction blocks are
also valid.

An active selection has precedence and is the only content sent or replaceable.
Otherwise only `<catblock>` interiors are sent; combined replacements are one
atomic transaction, while queued replacements run strictly serially and are
separately undoable. Without either, F3 uses the whole retained file: more than
the configured soft line threshold requires a typed one-time `yes`, while the
2,000-line / 64-KiB hard ceiling cannot be overridden.

Instruction cleanup defaults on and is part of the visible proposal and the
same accepted transaction. Failures, cancellation, rejection, drift, or a
partial queue keep the instruction. Applied content receives semantic
`llm_changed` presentation metadata; deleted and cleanup lines retain a gutter
marker. The metadata never changes file bytes and is cleared by undo, an
invalidating ordinary edit, the next clanker apply, buffer close, or the
`clear-clanker-changes` action. Color-disabled rendering uses underline/reverse
video plus the gutter marker.

## Construction / Invocation

- `F10`, `:model`, and `:models` load only validated preset metadata. Opening,
  filtering, or selecting in the picker must not construct a client, read a
  credential value, contact an endpoint, run a version probe, or start a child.
- The configured default and any process-local session override are separate.
  Selection never persists configuration and never invokes the backend.
- Network LLM clients and command processes must only be constructed after
  explicit invocation and Enter confirmation naming preset, adapter, exact
  destination identity, model, and context extent.
- Endpoint configuration is parsed and canonicalized before confirmation;
  credentials, whitespace, queries, fragments, and non-HTTP(S) schemes fail.
- API keys and credential headers must never cross non-loopback plaintext HTTP.
  Loopback HTTP may use credentials, and unauthenticated LAN HTTP remains
  available for local models.
- The transient HTTP client must not follow redirects away from the confirmed
  endpoint; every 3xx response is an error.
- Ambient proxy environment variables must not reroute context. Proxy support
  requires future explicit configuration and confirmation.
- Startup and ordinary editing must not gain model machinery or unconfirmed
  network work.
- Provider headers are explicit per preset. Static headers are non-secret
  metadata; credential-looking static headers are rejected in favor of named
  environment variables. Values are scoped to that preset; secret values are
  read only after send or discovery confirmation and are never rendered or
  copied to another preset. Static and environment-sourced values are valid,
  bounded HTTP header values.
- Model discovery is disabled unless configured for that HTTP preset and still
  requires `Ctrl+D` plus Enter in the picker. It sends no file context, follows
  no redirect, is cancellable, uses at most a ten-second timeout, and caps the
  response at 256 KiB/128 validated identifiers before keeping a five-minute
  process-local cache.
- Command presets keep program and argv separate and add no implicit `/bin/sh -c`.
  Catomic resolves the executable before confirmation, writes the versioned
  prompt transcript to stdin, starts the child in a private temporary working
  directory, caps stdout at 2 MiB and stderr at 64 KiB, enforces the configured
  timeout, and kills the complete child process group while reaping its direct
  child on cancellation.
- Current-buffer and inline prompts use only the active-file basename. Neither
  HTTP nor command payloads include Catomic's absolute workspace path, and
  command children do not inherit Catomic's cwd.
- Command stdout must match exactly `claude-json-v1` or `codex-jsonl-v1`.
  Malformed/partial output and Codex tool/item events fail closed. Stderr and
  HTTP error bodies are suppressed rather than copied into terminal errors.
  Backend output containing terminal control characters also fails closed.
- Catomic loads command presets only from the user configuration file. It does
  not accept repository-local command configuration. A configured executable is
  still user-trusted code with the user's OS permissions; use only a verified
  non-interactive text/proposal mode with tools and workspace mutation disabled.
- All patches go through `llm/patch.rs` and the read-only preview path.
- Current-buffer requests pin the active path through confirmation and response;
  path drift discards the request/output and patch headers must match that path.
- Inline requests also pin the exact instruction, selected ranges, block
  delimiters, revision, and path before send, preview, and apply. Queued work
  revalidates before every request and Escape cancels the active request and all
  remaining work.
- Tests use loopback fake HTTP only; never test against a live endpoint.
