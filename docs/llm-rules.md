# LLM Rules

No silent writes. No blind full-file replacement. No hidden network. No automatic repo upload.

## Output Preference Order

1. unified diff/patch
2. marked region
3. full file (explicit confirm only)

Every LLM edit must be previewed, confirmed, undoable.

## Commands

- `:meow` — selection/block (Plain allowed when explicit)
- `:bigmeow` — current file
- `:gitmeow` / `:megameow` — repo-aware (Project only)
- `:feralmeow` — wide (still preview-only)

## Repo LLM

Repo LLM must use a broker with context budget + read-only access.

Snapshot HEAD + branch + dirty state before calls.

If files change during thinking, refuse blind apply.

## Construction / Invocation

- Network LLM clients must only be constructed on explicit user invocation.
- Plain mode must not gain background network or repo LLM machinery.
- All patches go through `llm/patch.rs` preview path.
