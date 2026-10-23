# Decision 0017: One destructive confirmation at a time

Date: 2026-10-18

Status: accepted

## Context

Quit, save-conflict, reload, and config-discard confirmations were four
independent fields. Nothing stopped several from being armed together, so the
mobile bar and status code had to exclude the others by hand. The save-conflict
and reload records stored a full `ExternalFileStatus`, so path-less and
unchanged files were representable and their message functions carried
fallback arms for them. A boolean separated a passive watcher observation from
an armed reload, so an "armed but unknown" reload could also be represented.

## Decision

`App::confirmation` is one `Confirmation`: none, quit, a save conflict, an
armed reload, or a config discard. A save conflict records a
`ConflictingChange` (modified with its exact snapshot, deleted, or an unknown
status). An armed reload records a `ReloadableChange` (modified or deleted
only). Passive watcher observations live in `App::observed_change`, are swapped
with each buffer, and never arm anything.

## Consequences

- Arming one confirmation disarms the previous one by construction.
- Message functions match only the changes their confirmation can hold.
- An unrelated action resets `confirmation` but keeps the observed change, so
  the footer still shows that the file differs from disk.
