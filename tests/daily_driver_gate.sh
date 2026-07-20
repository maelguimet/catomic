#!/usr/bin/env bash
# Purpose: regression-test the daily-driver acceptance record validator.
# Owns: complete, incomplete, deferred-child, and tampered-binary fixtures.
# Must not: build or launch Catomic, use a real terminal, or retain temporary evidence.
# Invariants: every fixture is private, local, and removed on exit.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
gate="$repo_root/scripts/daily-driver-gate.sh"
temp_dir="$(mktemp -d)"
trap 'rm -rf -- "$temp_dir"' EXIT

make_session() {
  local session="$1"
  mkdir -p "$session/release"
  printf '%s\n' 'preserved release bytes' > "$session/release/catomic"
  local digest
  digest="$(sha256sum "$session/release/catomic" | cut -d ' ' -f 1)"
  cat > "$session/session.env" <<EOF
schema_version=1
source_sha=0123456789abcdef0123456789abcdef01234567
branch=codex/issue-72
release_version=catomic 0.1.0-beta.1
release_binary_sha256=$digest
terminal=Kitty 0.42 猫
term_environment=xterm-kitty
multiplexer=none
locale=C.UTF-8
locale_charmap=UTF-8
initial_dimensions=24 80
started_utc=2026-07-18T00:00:00Z
ended_utc=2026-07-18T01:00:00Z
duration_seconds=3600
editor_exit_status=0
terminal_state_restored=yes
EOF
  cat > "$session/final-comment.md" <<EOF
# Open-beta daily-driver acceptance

## Candidate and environment

- Result: PASS
- Source SHA: \`0123456789abcdef0123456789abcdef01234567\`
- Branch: \`codex/issue-72\`
- Release binary: \`catomic 0.1.0-beta.1\`
- Release binary SHA-256: \`$digest\`
- Terminal: Kitty 0.42 猫
- TERM: \`xterm-kitty\`
- Multiplexer: none
- Locale: \`C.UTF-8\` (\`UTF-8\`)
- Initial dimensions: \`24 80\`
- Dimensions exercised: 80x24 and 120x40
- Started (UTC): \`2026-07-18T00:00:00Z\`
- Ended (UTC): \`2026-07-18T01:00:00Z\`
- Session duration: 3600 seconds
- Editor exit status: \`0\`
- Exact terminal settings restored: \`yes\`

## Child issue dispositions

- [x] #64 — https://github.com/maelguimet/catomic/pull/164
- [x] #63 — https://github.com/maelguimet/catomic/pull/163
- [x] #56 — https://github.com/maelguimet/catomic/pull/156
- [x] #57 — https://github.com/maelguimet/catomic/pull/157
- [x] #54 — Deferred: Table alignment remains bounded to the documented beta behavior.
- [x] #55 — Deferred: Viewport scrolling remains scheduled after cursor correctness.

## Cold-use discoverability

- [x] Used only built-in help for open → edit → undo/redo → find → save → close.
- [x] Common actions were understandable without repository documentation.

## Sustained editing

- [x] Created, opened, saved, Save-As'd, and closed real text and code files.
- [x] Exercised undo/redo, selection, search/replace, clipboard, and wrapping.
- [x] Exercised multiple buffers, external changes, and catnap recovery.
- [x] Observed no data loss, stuck input, unexplained state, or terminal corruption.
- [x] Compared intended saved content with workspace files and after.sha256.

## Visual states and widths

- [x] Normal status remained distinct from document text.
- [x] Prompt, warning/confirmation, and error states were visibly distinct.
- [x] Status and prompts remained understandable at narrow and normal widths.
- [x] Markdown source and F6 preview were reviewed at both widths.

## Model safety

- [x] Opened current-file and repository model confirmations, then pressed Escape.
- [x] Confirmed no model request or live endpoint was used.

## Defects

- None observed.

## Remaining limitations

- Deferred behavior is limited to the two child dispositions above.

## Result

- This exact candidate passed the recorded manual gate.
EOF
}

complete="$temp_dir/complete"
make_session "$complete"
"$gate" verify "$complete" >/dev/null

incomplete="$temp_dir/incomplete"
cp -a "$complete" "$incomplete"
sed -i 's/- Result: PASS/- Result: TODO/' "$incomplete/final-comment.md"
if "$gate" verify "$incomplete" >/dev/null 2>&1; then
  echo "incomplete record unexpectedly passed" >&2
  exit 1
fi

missing_scenario="$temp_dir/missing-scenario"
cp -a "$complete" "$missing_scenario"
sed -i '/Common actions were understandable without repository documentation./d' \
  "$missing_scenario/final-comment.md"
if "$gate" verify "$missing_scenario" >/dev/null 2>&1; then
  echo "record with a deleted scenario unexpectedly passed" >&2
  exit 1
fi

short_deferral="$temp_dir/short-deferral"
cp -a "$complete" "$short_deferral"
sed -i 's/Deferred: Table alignment remains bounded to the documented beta behavior./Deferred: later/' \
  "$short_deferral/final-comment.md"
if "$gate" verify "$short_deferral" >/dev/null 2>&1; then
  echo "unexplained child deferral unexpectedly passed" >&2
  exit 1
fi

short_session="$temp_dir/short-session"
cp -a "$complete" "$short_session"
sed -i 's/duration_seconds=3600/duration_seconds=479/' "$short_session/session.env"
sed -i 's/Session duration: 3600 seconds/Session duration: 479 seconds/' \
  "$short_session/final-comment.md"
if "$gate" verify "$short_session" >/dev/null 2>&1; then
  echo "short daily-driver session unexpectedly passed" >&2
  exit 1
fi

tampered="$temp_dir/tampered"
cp -a "$complete" "$tampered"
printf '%s\n' 'tampered' >> "$tampered/release/catomic"
if "$gate" verify "$tampered" >/dev/null 2>&1; then
  echo "tampered release binary unexpectedly passed" >&2
  exit 1
fi

echo "daily-driver gate validator tests passed"
