# Security Policy

Catomic edits local files, runs explicitly configured commands and hooks, and
can contact GitHub through its explicit updater. A vulnerability in any of
those boundaries may expose or destroy user data, so please report suspected
security issues privately.

## Supported versions

During open beta, security fixes target the latest published release and the
current `master` branch. Older snapshots may be asked to reproduce on the latest
version before a fix is prepared.

## Report a vulnerability

Use GitHub's private
[Report a vulnerability](https://github.com/maelguimet/catomic/security/advisories/new)
flow. Do not open a public issue for an unpatched vulnerability, and do not put
private document contents, API keys, recovery files, or other secrets in a
report.

Include as much of the following as you safely can:

- the Catomic version or commit;
- Linux distribution, kernel, terminal, and filesystem or mount type;
- a minimal reproduction using non-sensitive sample data;
- the expected and observed behavior;
- the security impact, including whether confidentiality, integrity, or
  availability is affected;
- whether symlinks, hard links, unusual file types, external commands, hooks,
  or the updater are involved; and
- any proposed fix or mitigation.

Please allow time to reproduce and fix the issue before public disclosure. The
maintainer will coordinate disclosure and credit with the reporter after the
fix is available.

## Non-sensitive safety bugs

Crashes, data-loss reports, terminal corruption, and filesystem surprises are
important even when they are not exploitable. If a report can be shared safely,
use the repository's
[bug report form](https://github.com/maelguimet/catomic/issues/new?template=bug_report.yml)
and replace private content with a minimal fixture.

## Scope and security model

- Editor-owned paths have no built-in networking. Catomic has no built-in model
  provider, AI prompt/runtime, or repository-aware assistant.
- Linting and external commands run only after their explicit actions.
- `catomic update` may contact the documented GitHub source after an explicit
  updater invocation. Its downloaded candidate, source identity, and checksum
  checks are security boundaries.
- Linters, commands, and hooks in the user's configuration are trusted local
  code and run through `/bin/sh -c`; arbitrary side effects, including network
  access, from code the user configured are not a sandbox escape.
- Silent network access added to editor-owned paths, updater origin or
  verification bypasses, and command execution without the documented explicit
  trigger are in scope.
