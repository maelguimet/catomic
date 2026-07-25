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

Wide or multi-file patches are not accepted.

## Construction / Invocation

- The configured `llm.default` names the preset used for every request. Changing
  it requires an explicit configuration edit; there is no picker or
  process-local override. The catalog is read only after explicit invocation.
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
  read only after send confirmation and are never rendered or
  copied to another preset. Static and environment-sourced values are valid,
  bounded HTTP header values.
- Command presets keep program and argv separate and add no implicit `/bin/sh -c`.
  Catomic resolves the executable before confirmation, writes the versioned
  prompt transcript to stdin, starts the child in a private temporary working
  directory, caps stdout at 2 MiB and stderr at 64 KiB, enforces the configured
  timeout, and kills the complete child process group while reaping its direct
  child on cancellation.
- Current-buffer prompts use only the active-file basename. Neither
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
- Tests use loopback fake HTTP only; never test against a live endpoint.
