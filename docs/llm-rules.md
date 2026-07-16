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
- `:gitmeow` / `:megameow` — repo-aware (Project only)

`:feralmeow` remains unimplemented: Phase 6 does not accept wide or multi-file
patches.

## Repo LLM

Repo LLM must use a broker with context budget + read-only access.

Snapshot HEAD + branch + dirty state before calls.

If files change during thinking or before preview apply, refuse blind apply.

Broker commands are limited to list files, bounded ranged reads, bounded grep,
and per-file diff. No command writes or runs a process other than read-only Git.

## Construction / Invocation

- Network LLM clients must only be constructed after explicit invocation and
  Enter confirmation naming endpoint, model, and context extent.
- Plain mode must not gain background network or repo LLM machinery.
- All patches go through `llm/patch.rs` and the read-only preview path.
- Tests use loopback fake HTTP only; never test against a live endpoint.
