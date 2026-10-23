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

The [GitHub issue queue](https://github.com/maelguimet/catomic/issues) is the
authority for active work and priorities. Acceptance records in [`docs/`](docs/)
preserve verification evidence; historical plans under `docs/progress/` are not
active requirements. Read [AGENTS.md](AGENTS.md) before changing code; its
architecture, testing, and scope rules apply to human and automated contributors
alike.

## Development setup

Catomic currently targets Linux, stable Rust, and Python 3.11 or newer for the
repository source gates.

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
4. Preserve the construction and typing boundary: no implicit repository scans,
   network clients, or persistent workspace services, and no subprocess except
   after an explicit action or configured lifecycle event.
5. Never make verification depend on a live public endpoint or credential.
6. Update user-facing documentation when commands, configuration, safety
   behavior, or limitations change.

The architecture overview is in [docs/architecture.md](docs/architecture.md),
performance rules are in [docs/performance.md](docs/performance.md), and the
no-built-in-AI boundary is recorded in
[decision 0015](docs/decisions/0015-no-built-in-ai-runtime.md).

## Verify your change

Run the normal local gate before submitting a pull request:

```sh
python3 scripts/check_markdown_links.py
python3 scripts/check_no_builtin_ai.py
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

Maintainers can manually dispatch the separate
[**Acceptance** GitHub Actions workflow](.github/workflows/acceptance.yml).
It runs ignored checks and release-mode byte-scan benchmarks, then builds one
managed candidate binary and exercises direct-PTY, tmux, runner-filesystem, and
tmpfs compatibility paths. The run retains the binary, checksum, JSON results,
and partial automated matrix in its `catomic-compatibility-<commit SHA>`
artifact for 30 days.
Complete the remaining [Linux compatibility matrix](docs/compatibility.md)
requirements on that same binary and publish durable evidence before release,
as described in the [release procedure](docs/releasing.md).

Maintainers manually dispatch
[**Publish managed release**](.github/workflows/release.yml) with the version
and originating Acceptance run ID. It checks and promotes the exact accepted
managed bytes, creates the immutable version tag, runs source, package, and
public-asset checks, and retains the partial automated matrix with the release.
The separately published manual release-candidate matrix remains the complete
terminal/filesystem compatibility record.
Some ignored checks create large temporary fixtures or measure live terminal
and filesystem behavior. Read the relevant acceptance record under `docs/`
before diagnosing an environment-sensitive result. Tests and compatibility
runners must not contact live public services.

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

## File-size cap

Each file stays under about 5,000 tokens, counted as 20,000 bytes. CI is the
gate: the File size workflow runs `scripts/check-file-size --all` on every push
and pull request. On pull requests it runs the base branch's copy of the
script, and also rejects new or raised `.file-size-baseline` entries and any
over-cap file that is exempt through `.gitattributes` but was not an exempt
file on the base branch, including a new file under an existing exemption.
Narrowing an exemption passes. The pre-commit hook runs the same check on
staged files before CI does. Enable it once per clone:

```sh
git config core.hooksPath .githooks
```

`scripts/check-file-size --all` checks the whole tree. When the check fails,
split the file by responsibility, move tests to their own file, or extract data
and fixtures. Some files were already over the cap when the check was added.
They are listed in `.file-size-baseline` with their size at that time; they may
shrink but not grow, and their entries may only be lowered or removed.
Lockfiles (by exact name), files with binary content, and paths marked
`linguist-generated`, `linguist-vendored`, `binary` or `-diff` in the committed
`.gitattributes` are exempt.
