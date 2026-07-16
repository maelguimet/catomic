# Phase 8 Progress

Phase 8 is complete. Its exit evidence is in
[`../phase-8-acceptance.md`](../phase-8-acceptance.md).

## Completed

- **Status polish**: a default-on ASCII cat badge with an exact presentation-only
  opt-out.
- **Useful cat command**: the existing `:meow` remains the explicit, confirmed,
  previewed LLM command instead of gaining a second ambiguous behavior.
- **Panic notice**: terminal restoration precedes a concise cat-themed message,
  then the normal diagnostic hook runs.
- **Private catnaps**: opt-in typed configuration, bounded content and intervals,
  append-only sidecar naming, 0600 atomic writes, symlink refusal, and capped
  UTF-8 reads.
- **Preview-first recovery**: startup offer, explicit `:recover`, read-only view,
  stale-source refusal, one-step undo, and no source write before ordinary save.
- **Lifecycle safety**: per-buffer workers and timers, save/write race closure,
  sidecar cleanup only after successful save, and zero recovery writes by default.
- **Terminal and performance acceptance**: one real PTY recovery round trip and a
  measured maximum-default 1 MiB write/read workload.

## Deliberate boundary

Catnap recovery is not a session journal. Untitled, paged, and oversized buffers
are deliberately skipped; cursor history and undo stacks are not serialized.
Catomic never auto-applies a sidecar and never claims unsaved work survived a
panic. Those limits keep recovery obvious and prevent a background storage
system from becoming a second editor core.
