# Contributing to Catomic

Thanks for helping make Catomic safer and more useful. The project values small,
reviewable changes, measurable behavior, and boring core code over feature
volume.

## Before you start

- Search existing issues and pull requests before opening a duplicate.
- Use the
  [bug report form](https://github.com/maelguimet/catomic/issues/new?template=bug_report.yml)
  for reproducible bugs.
- Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
- For a substantial feature or architecture change, open an issue first so its
  scope and fit can be discussed.

The active roadmap authority is [TODO.md](TODO.md) together with the acceptance
records in [`docs/`](docs/). Historical plans under `docs/progress/` preserve
design evidence but are not active requirements. Read [AGENTS.md](AGENTS.md)
before changing code; its architecture, testing, and scope rules apply to human
and automated contributors alike.

## Development setup

Catomic currently targets Linux and stable Rust.

```sh
git clone https://github.com/maelguimet/catomic.git
cd catomic
rustup toolchain install stable --profile minimal --component clippy,rustfmt
cargo build --release --locked
```

Do not update `Cargo.lock` unless the change intentionally updates
dependencies. Every new dependency needs the justification required by
[AGENTS.md](AGENTS.md).

## Make a focused change

1. Start from a clean branch based on current `master`.
2. Keep each commit to one coherent change; avoid drive-by formatting.
3. Add regression tests for behavior changes. Core buffer, coordinate, and
   undo/redo work should be developed test-first.
4. Preserve Plain mode's boundary: no implicit repository scans, subprocesses,
   network clients, or Project services.
5. Never run tests against a live model or public endpoint.
6. Update user-facing documentation when commands, configuration, safety
   behavior, or limitations change.

The architecture overview is in [docs/architecture.md](docs/architecture.md),
performance rules are in [docs/performance.md](docs/performance.md), and LLM
boundaries are in [docs/llm-rules.md](docs/llm-rules.md).

## Verify your change

Run the normal local gate before submitting a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
cargo package --locked --list
git diff --check
```

Ignored and environment-sensitive checks are intentionally excluded from normal
pull-request CI. Run them serially for release candidates:

```sh
cargo test --all-targets --locked -- --ignored --test-threads=1 --nocapture
```

Maintainers can also run the separate **Acceptance** GitHub Actions workflow
manually; version-tag pushes run it automatically. Some ignored checks create
large temporary fixtures or measure live terminal and filesystem behavior.
Read the relevant acceptance record under `docs/` before diagnosing an
environment-sensitive result. No verification step may contact a live model or
public endpoint.

## Pull requests

In the pull request description:

- explain the user-visible problem and the chosen behavior;
- identify important safety or performance tradeoffs;
- list the exact checks run and any checks that were not run;
- include terminal recordings or screenshots for visible UI changes when
  practical; and
- call out filesystem, terminal, or locale assumptions.

Keep unrelated changes in separate pull requests. A reviewer should be able to
understand the change without reconstructing a hidden roadmap from the diff.
