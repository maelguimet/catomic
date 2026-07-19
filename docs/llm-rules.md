# LLM Rules

No silent writes. No blind full-file replacement. No hidden network. No automatic repo upload.

## Output Preference Order

1. unified diff/patch
2. marked region

Full-file replacement output is not accepted. A marked region uses only the
strict `{"catomic_replacement":"..."}` envelope.

Every LLM edit must be previewed, confirmed, undoable.

## Commands

- `:meow` — selection/block (Plain allowed when explicit)
- `:bigmeow` — current file
- `:gitmeow` — focused repo-aware request, capped at 64 KiB of broker context (Project only)
- `:megameow` — broader repo-aware request, capped at 128 KiB of broker context (Project only)

`:feralmeow` remains unimplemented: Phase 6 does not accept wide or multi-file
patches.

## Repo LLM

Repo LLM must use a broker with context budget + read-only access.

Snapshot HEAD + branch + dirty state before calls.

If files change during thinking or before preview apply, refuse blind apply.

Read-only Git capture must disable pagers, fsmonitor, external diff, and
textconv helpers so repository configuration cannot launch child helpers. It
must strip inherited `GIT_*` variables before applying its safe settings. Git
stdin is closed; every child has a ten-second timeout and is killed and reaped
when its owning preparation, request, or drift-check worker is cancelled.

Broker commands are limited to list files, bounded ranged reads, bounded grep,
and per-file diff. No command writes or runs a process other than read-only Git.

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
- Plain mode must not gain background network or repo LLM machinery.
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
- Command prompts use an active-file basename or confirmed repo-relative path,
  not the workspace's absolute path; the child does not inherit Catomic's cwd.
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
- Repo requests pin the active path through context preparation, confirmation,
  response, and final preview apply; path drift cancels or discards fail closed.
- The confirmed repo pre-send drift check runs on a pollable worker; Enter and
  ordinary editor polling must never run Git on the input thread.
- The repo request worker rechecks drift after the final response before handing
  output back to the editor; response polling must never run Git.
- Final Enter on a repo preview starts another pollable drift worker. The preview
  stays read-only, and only an unchanged result reaches the undoable apply path.
- Repo preparation fingerprints the active file on disk even when it is
  untracked, so byte drift hidden from Git status is refused at every send/apply gate.
- The first relevant-file fingerprint is immutable for the request; later
  broker reads or grep cannot refresh a drifted baseline.
- Broker reads hash and expose one bounded opened-file snapshot, with canonical
  in-repo path and pre/post file-revision checks; they never hash then reopen.
- The repo broker omits dot paths, refuses direct reads or diffs containing
  obvious secret-like content, and makes grep skip and count sensitive files.
  An explicitly confirmed active dotfile remains governed by active-context
  sensitivity confirmation rather than broker retrieval.
- Repo patch headers must name the exact active repo-relative file; patches for
  another file and rename-shaped patches fail before preview.
- Tests use loopback fake HTTP only; never test against a live endpoint.
