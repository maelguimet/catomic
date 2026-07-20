# Decision 0009: Parse Configuration as TOML

Status: accepted for Phase 7

## Context

Catomic already documents `config.toml`, but completed phases accumulated four
small section-specific parsers. They recognize only a TOML-shaped subset and
disagree with TOML on quoting, comments, escapes, and dotted keys. Phase 7 adds
language settings, keybindings, commands, and hooks, so extending those parsers
would create ambiguous configuration and duplicate validation.

## Decision

Use the `toml` crate with the existing `serde` dependency to decode typed
configuration sections. Unknown fields remain ignored for forward
compatibility, while malformed TOML and invalid recognized values fail with an
`InvalidData` error. Missing files continue to use safe defaults.

The standard library has no TOML parser. This dependency is used for Plain-safe
startup settings and for lazily loaded repository/LLM settings; parsing data does
not construct a linter, repository scanner, process runner, network client, or
other capability-gated service. Unit tests cover defaults, valid TOML syntax,
section isolation, and validation failures.

If configuration later moves to another format, removal is localized to the
`config` module and its typed section decoders. Disabling optional sections does
not require disabling the parser or changing Plain-mode service construction.
