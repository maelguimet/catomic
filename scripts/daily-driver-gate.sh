#!/usr/bin/env bash
# Purpose: run and validate the human open-beta daily-driver acceptance session.
# Owns: clean-SHA release binding, isolated fixtures, terminal metadata, and comment schema.
# Must not: claim human scenarios passed, contact a model, publish evidence, or overwrite a session.
# Invariants: the tested binary is copied before launch and every PASS record is complete.

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
minimum_session_seconds=480

die() {
  echo "daily-driver gate failed: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'EOF'
usage:
  scripts/daily-driver-gate.sh run SESSION_DIR --terminal "NAME VERSION" --multiplexer "NAME VERSION|none"
  scripts/daily-driver-gate.sh verify SESSION_DIR

run requires an interactive terminal, a clean checkout, and a new SESSION_DIR.
verify accepts only a completed PASS record; it never edits the session.
EOF
  exit 2
}

require_tool() {
  command -v "$1" >/dev/null || die "required tool is missing: $1"
}

require_single_line() {
  local label="$1"
  local value="$2"
  [[ -n "$value" ]] || die "$label must not be empty"
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* && "$value" != *'`'* ]] ||
    die "$label must be one Markdown-safe line without backticks"
  ! LC_ALL=C printf '%s' "$value" | grep -q '[[:cntrl:]]' ||
    die "$label must not contain control characters"
}

session_file() {
  local path="$1"
  [[ -f "$path" && ! -L "$path" ]] || die "missing regular session file: $path"
}

manifest_value() {
  local manifest="$1"
  local key="$2"
  local count
  count="$(grep -c "^${key}=" "$manifest" || true)"
  [[ "$count" == 1 ]] || die "manifest must contain exactly one $key value"
  sed -n "s/^${key}=//p" "$manifest"
}

require_record_line() {
  local record="$1"
  local expected="$2"
  grep -Fqx -- "$expected" "$record" || die "record differs from captured evidence: $expected"
}

section_content() {
  local record="$1"
  local heading="$2"
  awk -v heading="$heading" '
    $0 == heading { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
  ' "$record"
}

verify_child_dispositions() {
  local record="$1"
  local issue prefix line disposition rationale
  for issue in 64 63 56 57 54 55; do
    prefix="- [x] #$issue — "
    line="$(grep -F -- "$prefix" "$record" || true)"
    [[ "$(grep -Fc -- "$prefix" "$record" || true)" == 1 ]] ||
      die "issue #$issue needs exactly one checked disposition"
    disposition="${line#"$prefix"}"
    if [[ "$disposition" =~ ^https://github\.com/maelguimet/catomic/pull/[0-9]+([[:space:]].*)?$ ]]; then
      continue
    fi
    [[ "$disposition" == "Deferred: "* ]] ||
      die "issue #$issue needs a closing PR URL or an explicit Deferred: rationale"
    rationale="${disposition#Deferred: }"
    [[ ${#rationale} -ge 20 ]] || die "issue #$issue deferral rationale is too short"
  done
}

verify_defects() {
  local record="$1"
  local defects line count=0
  defects="$(section_content "$record" "## Defects" | sed '/^$/d')"
  [[ -n "$defects" ]] || die "Defects must say None observed. or link focused issues"
  [[ "$defects" != *$'\n'* && "$defects" == "- None observed." ]] && return
  while IFS= read -r line; do
    [[ "$line" =~ ^-\ https://github\.com/maelguimet/catomic/issues/[0-9]+\ —\ .+ ]] ||
      die "each observed defect must link one focused issue and include evidence"
    count=$((count + 1))
  done <<< "$defects"
  [[ $count -gt 0 ]] || die "Defects section is empty"
}

verify_scenarios() {
  local record="$1"
  local scenario
  while IFS= read -r scenario; do
    require_record_line "$record" "- [x] $scenario"
  done <<'EOF'
Used only built-in help for open → edit → undo/redo → find → save → close.
Common actions were understandable without repository documentation.
Created, opened, saved, Save-As'd, and closed real text and code files.
Exercised undo/redo, selection, search/replace, clipboard, and wrapping.
Exercised multiple buffers, external changes, and catnap recovery.
Observed no data loss, stuck input, unexplained state, or terminal corruption.
Compared intended saved content with workspace files and after.sha256.
Normal status remained distinct from document text.
Prompt, warning/confirmation, and error states were visibly distinct.
Status and prompts remained understandable at narrow and normal widths.
Markdown source and F6 preview were reviewed at both widths.
Opened current-file and repository model confirmations, then pressed Escape.
Confirmed no model request or live endpoint was used.
EOF
}

verify_captured_metadata() {
  local manifest="$1" record="$2" digest="$3"
  local source_sha release_version duration branch terminal term multiplexer locale charmap
  local dimensions started ended exit_status restored expected
  source_sha="$(manifest_value "$manifest" source_sha)"
  release_version="$(manifest_value "$manifest" release_version)"
  duration="$(manifest_value "$manifest" duration_seconds)"
  branch="$(manifest_value "$manifest" branch)"
  terminal="$(manifest_value "$manifest" terminal)"
  term="$(manifest_value "$manifest" term_environment)"
  multiplexer="$(manifest_value "$manifest" multiplexer)"
  locale="$(manifest_value "$manifest" locale)"
  charmap="$(manifest_value "$manifest" locale_charmap)"
  dimensions="$(manifest_value "$manifest" initial_dimensions)"
  started="$(manifest_value "$manifest" started_utc)"
  ended="$(manifest_value "$manifest" ended_utc)"
  exit_status="$(manifest_value "$manifest" editor_exit_status)"
  restored="$(manifest_value "$manifest" terminal_state_restored)"

  [[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || die "manifest source SHA is invalid"
  [[ "$duration" =~ ^[1-9][0-9]*$ ]] || die "session duration must be positive"
  ((duration >= minimum_session_seconds)) ||
    die "session duration must be at least $minimum_session_seconds seconds"
  printf -v expected -- "- Source SHA: \`%s\`" "$source_sha"
  require_record_line "$record" "$expected"
  require_record_line "$record" "- Branch: \`$branch\`"
  printf -v expected -- "- Release binary: \`%s\`" "$release_version"
  require_record_line "$record" "$expected"
  printf -v expected -- "- Release binary SHA-256: \`%s\`" "$digest"
  require_record_line "$record" "$expected"
  require_record_line "$record" "- Terminal: $terminal"
  require_record_line "$record" "- TERM: \`$term\`"
  require_record_line "$record" "- Multiplexer: $multiplexer"
  require_record_line "$record" "- Locale: \`$locale\` (\`$charmap\`)"
  require_record_line "$record" "- Initial dimensions: \`$dimensions\`"
  require_record_line "$record" "- Started (UTC): \`$started\`"
  require_record_line "$record" "- Ended (UTC): \`$ended\`"
  require_record_line "$record" "- Session duration: $duration seconds"
  require_record_line "$record" "- Editor exit status: \`$exit_status\`"
  require_record_line "$record" "- Exact terminal settings restored: \`$restored\`"
}

verify_record() {
  [[ $# -eq 1 ]] || usage
  local session_dir="$1"
  [[ -d "$session_dir" && ! -L "$session_dir" ]] || die "session directory is invalid"
  session_dir="$(realpath -- "$session_dir")"
  local manifest="$session_dir/session.env"
  local record="$session_dir/final-comment.md"
  local binary="$session_dir/release/catomic"
  session_file "$manifest"
  session_file "$record"
  session_file "$binary"

  local expected_digest actual_digest source_sha release_version
  [[ "$(manifest_value "$manifest" schema_version)" == 1 ]] || die "unsupported session schema"
  expected_digest="$(manifest_value "$manifest" release_binary_sha256)"
  [[ "$expected_digest" =~ ^[0-9a-f]{64}$ ]] || die "manifest binary digest is invalid"
  actual_digest="$(sha256sum -- "$binary" | cut -d ' ' -f 1)"
  [[ "$actual_digest" == "$expected_digest" ]] || die "preserved release binary digest changed"
  [[ "$(manifest_value "$manifest" editor_exit_status)" == 0 ]] || die "editor did not exit successfully"
  [[ "$(manifest_value "$manifest" terminal_state_restored)" == yes ]] ||
    die "terminal settings were not restored exactly"

  source_sha="$(manifest_value "$manifest" source_sha)"
  release_version="$(manifest_value "$manifest" release_version)"
  verify_captured_metadata "$manifest" "$record" "$expected_digest"
  require_record_line "$record" "- Result: PASS"

  local heading
  for heading in \
    "## Candidate and environment" \
    "## Child issue dispositions" \
    "## Cold-use discoverability" \
    "## Sustained editing" \
    "## Visual states and widths" \
    "## Model safety" \
    "## Defects" \
    "## Remaining limitations" \
    "## Result"; do
    [[ "$(grep -Fxc -- "$heading" "$record" || true)" == 1 ]] ||
      die "record needs exactly one $heading"
  done
  ! grep -Fq -- "TODO" "$record" || die "record still contains TODO placeholders"
  ! grep -Eq '^- \[ \]' "$record" || die "record still contains unchecked acceptance items"
  verify_child_dispositions "$record"
  verify_scenarios "$record"
  verify_defects "$record"

  local dimensions limitations result
  dimensions="$(grep -E '^- Dimensions exercised: .+' "$record" || true)"
  [[ "$(grep -Ec '^- Dimensions exercised: .+' "$record" || true)" == 1 &&
    ${#dimensions} -ge 27 ]] || die "Dimensions exercised needs one concrete value"
  limitations="$(section_content "$record" "## Remaining limitations" | sed '/^$/d')"
  [[ "$limitations" == "- "* && ${#limitations} -ge 12 ]] ||
    die "Remaining limitations needs a concrete bullet"
  result="$(section_content "$record" "## Result" | sed '/^$/d')"
  [[ "$result" == "- "* && ${#result} -ge 12 ]] || die "Result needs a concrete summary bullet"
  echo "daily-driver record verified for $release_version at $source_sha"
}

write_fixtures() {
  local session_dir="$1"
  local workspace="$session_dir/workspace"
  mkdir -p -- "$workspace" "$session_dir/config/catomic" "$session_dir/home"

  cat > "$session_dir/config/catomic/config.toml" <<'EOF'
[recovery]
enabled = true
interval_secs = 5
max_bytes = 1048576

[llm]
base_url = "http://127.0.0.1:9/v1"
model = "acceptance-no-send"
EOF
  cat > "$workspace/notes.txt" <<'EOF'
Daily-driver notes

Edit this prose through several paragraphs. Exercise selection, clipboard,
search, replacement, wrapping, undo, redo, Save As, and multiple buffers.

Unicode boundary fixture: café 猫 👩‍💻 and	a tab.
EOF
  cat > "$workspace/sample.rs" <<'EOF'
fn main() {
    let greeting = "hello from the release candidate";
    println!("{greeting}");
}
EOF
  cat > "$workspace/showcase.md" <<'EOF'
# Markdown acceptance

| Left | Center | Right |
| :--- | :----: | ----: |
| short | `code` | 10 |
| wide 猫 emoji 🐾 | a much longer value | 2,000 |

- [x] source remains editable
- [ ] preview remains read-only
EOF
  printf '%s\n' "external baseline" > "$workspace/external.txt"
  printf '%s\n' "saved recovery baseline" > "$workspace/recovery.txt"
  cat > "$workspace/external-change.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "external change at $(date -u +%Y-%m-%dT%H:%M:%SZ)" > \
  "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/external.txt"
EOF
  chmod 0700 "$workspace/external-change.sh"

  git -C "$workspace" init -q
  git -C "$workspace" add -- .
  git -C "$workspace" -c user.name='Catomic Acceptance' \
    -c user.email='acceptance@invalid.example' commit -qm 'Seed daily-driver fixtures'
  touch -d '2 minutes ago' "$workspace/recovery.txt"
  printf '%s\n' "recovered draft with Unicode 猫" > "$workspace/recovery.txt.catnap"
  (cd -- "$workspace" && sha256sum notes.txt sample.rs showcase.md external.txt recovery.txt) \
    > "$session_dir/before.sha256"
}

write_session_record() {
  local session_dir="$1" source_sha="$2" branch="$3" release_version="$4"
  local digest="$5" terminal_label="$6" term_value="$7" multiplexer="$8"
  local locale_value="$9" charmap="${10}" dimensions="${11}" started="${12}"
  local ended="${13}" duration="${14}" editor_status="${15}" restored="${16}"
  cat > "$session_dir/session.env" <<EOF
schema_version=1
source_sha=$source_sha
branch=$branch
release_version=$release_version
release_binary_sha256=$digest
terminal=$terminal_label
term_environment=$term_value
multiplexer=$multiplexer
locale=$locale_value
locale_charmap=$charmap
initial_dimensions=$dimensions
started_utc=$started
ended_utc=$ended
duration_seconds=$duration
editor_exit_status=$editor_status
terminal_state_restored=$restored
EOF
  cat > "$session_dir/final-comment.md" <<EOF
# Open-beta daily-driver acceptance

## Candidate and environment

- Result: TODO
- Source SHA: \`$source_sha\`
- Branch: \`$branch\`
- Release binary: \`$release_version\`
- Release binary SHA-256: \`$digest\`
- Terminal: $terminal_label
- TERM: \`$term_value\`
- Multiplexer: $multiplexer
- Locale: \`$locale_value\` (\`$charmap\`)
- Initial dimensions: \`$dimensions\`
- Dimensions exercised: TODO
- Started (UTC): \`$started\`
- Ended (UTC): \`$ended\`
- Session duration: $duration seconds
- Editor exit status: \`$editor_status\`
- Exact terminal settings restored: \`$restored\`

## Child issue dispositions

- [ ] #64 — TODO: closing PR URL or Deferred: concrete rationale
- [ ] #63 — TODO: closing PR URL or Deferred: concrete rationale
- [ ] #56 — TODO: closing PR URL or Deferred: concrete rationale
- [ ] #57 — TODO: closing PR URL or Deferred: concrete rationale
- [ ] #54 — TODO: closing PR URL or Deferred: concrete rationale
- [ ] #55 — TODO: closing PR URL or Deferred: concrete rationale

## Cold-use discoverability

- [ ] Used only built-in help for open → edit → undo/redo → find → save → close.
- [ ] Common actions were understandable without repository documentation.

## Sustained editing

- [ ] Created, opened, saved, Save-As'd, and closed real text and code files.
- [ ] Exercised undo/redo, selection, search/replace, clipboard, and wrapping.
- [ ] Exercised multiple buffers, external changes, and catnap recovery.
- [ ] Observed no data loss, stuck input, unexplained state, or terminal corruption.
- [ ] Compared intended saved content with workspace files and after.sha256.

## Visual states and widths

- [ ] Normal status remained distinct from document text.
- [ ] Prompt, warning/confirmation, and error states were visibly distinct.
- [ ] Status and prompts remained understandable at narrow and normal widths.
- [ ] Markdown source and F6 preview were reviewed at both widths.

## Model safety

- [ ] Opened current-file and repository model confirmations, then pressed Escape.
- [ ] Confirmed no model request or live endpoint was used.

## Defects

- TODO: replace with None observed. or one focused issue-link-and-evidence bullet per defect.

## Remaining limitations

- TODO: record concrete limitations, including any deferred child issue.

## Result

- TODO: summarize why this exact candidate passes or fails the gate.
EOF
}

run_session() {
  [[ $# -eq 5 ]] || usage
  local session_input="$1"
  [[ "$2" == --terminal && "$4" == --multiplexer ]] || usage
  local terminal_label="$3" multiplexer="$5"
  require_single_line "terminal label" "$terminal_label"
  require_single_line "multiplexer label" "$multiplexer"
  [[ -t 0 && -t 1 && -r /dev/tty && -w /dev/tty ]] ||
    die "run must be invoked from an interactive terminal"

  local tool
  for tool in awk cargo cut date git grep install locale realpath sed sha256sum stty touch; do
    require_tool "$tool"
  done
  [[ ! -e "$session_input" ]] || die "session path already exists: $session_input"
  [[ -d "$(dirname -- "$session_input")" ]] || die "session parent directory does not exist"
  [[ -z "$(git -C "$repo_root" status --porcelain=v1 --untracked-files=all)" ]] ||
    die "checkout must be clean so the release binary binds to one source SHA"

  local source_sha branch
  source_sha="$(git -C "$repo_root" rev-parse HEAD)"
  branch="$(git -C "$repo_root" branch --show-current)"
  [[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || die "checkout did not resolve to a full source SHA"
  require_single_line "branch" "$branch"

  local gate_target_dir="$repo_root/target/daily-driver-gate"
  (cd -- "$repo_root" && CARGO_TARGET_DIR="$gate_target_dir" cargo build --release --locked)
  local built_binary="$gate_target_dir/release/catomic"
  [[ -f "$built_binary" && ! -L "$built_binary" ]] || die "release binary was not built"
  [[ "$(git -C "$repo_root" rev-parse HEAD)" == "$source_sha" ]] ||
    die "checkout HEAD changed while the release binary was built"
  [[ -z "$(git -C "$repo_root" status --porcelain=v1 --untracked-files=all)" ]] ||
    die "checkout changed while the release binary was built"
  local release_version
  release_version="$($built_binary --version)"
  require_single_line "release version" "$release_version"

  umask 077
  mkdir -m 0700 -- "$session_input"
  local session_dir
  session_dir="$(realpath -- "$session_input")"
  mkdir -p -- "$session_dir/release"
  install -m 0500 -- "$built_binary" "$session_dir/release/catomic"
  local release_binary="$session_dir/release/catomic"
  local digest
  digest="$(sha256sum -- "$release_binary" | cut -d ' ' -f 1)"
  write_fixtures "$session_dir"

  local locale_value charmap term_value dimensions terminal_before
  locale_value="${LC_ALL:-${LC_CTYPE:-${LANG:-unset}}}"
  charmap="$(locale charmap)"
  [[ "$charmap" == UTF-8 || "$charmap" == utf8 ]] || die "session requires a UTF-8 locale"
  term_value="${TERM:-unset}"
  require_single_line "locale" "$locale_value"
  require_single_line "TERM" "$term_value"
  dimensions="$(stty size < /dev/tty)"
  terminal_before="$(stty -g < /dev/tty)"
  [[ "$dimensions" =~ ^[1-9][0-9]*\ [1-9][0-9]*$ ]] || die "terminal dimensions are unavailable"

  echo "Release $release_version ($source_sha) is preserved in $session_dir/release."
  echo "Use built-in help only for common actions. Exercise ordinary editing before model confirmations."
  echo "Run $session_dir/workspace/external-change.sh from another terminal for the external-change scenario."
  echo "Cancel every model confirmation with Escape; do not send a request."

  local started ended started_epoch ended_epoch editor_status terminal_after restored
  started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  started_epoch="$(date +%s)"
  set +e
  (
    cd -- "$session_dir/workspace"
    env -u OPENAI_API_KEY \
      HOME="$session_dir/home" \
      XDG_CONFIG_HOME="$session_dir/config" \
      "$release_binary" notes.txt sample.rs showcase.md external.txt recovery.txt
  )
  editor_status=$?
  set -e
  ended_epoch="$(date +%s)"
  ended="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  terminal_after="$(stty -g < /dev/tty 2>/dev/null || true)"
  restored=no
  [[ -n "$terminal_after" && "$terminal_after" == "$terminal_before" ]] && restored=yes
  local duration=$((ended_epoch - started_epoch))
  (cd -- "$session_dir/workspace" && \
    sha256sum notes.txt sample.rs showcase.md external.txt recovery.txt) \
    > "$session_dir/after.sha256"
  write_session_record "$session_dir" "$source_sha" "$branch" "$release_version" \
    "$digest" "$terminal_label" "$term_value" "$multiplexer" "$locale_value" \
    "$charmap" "$dimensions" "$started" "$ended" "$duration" "$editor_status" "$restored"

  [[ $editor_status -eq 0 ]] || die "editor exited with status $editor_status; evidence was retained"
  [[ "$restored" == yes ]] || die "terminal settings changed; evidence was retained"
  echo "Session evidence retained at $session_dir. Complete final-comment.md, then run:"
  echo "  scripts/daily-driver-gate.sh verify $session_dir"
}

[[ $# -ge 1 ]] || usage
command_name="$1"
shift
case "$command_name" in
  run) run_session "$@" ;;
  verify) verify_record "$@" ;;
  *) usage ;;
esac
