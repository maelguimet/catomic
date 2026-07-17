# Security Policy

Catomic edits local files, runs explicitly configured commands, and can send
selected text to an explicitly confirmed model endpoint. A vulnerability in any
of those boundaries may expose or destroy user data, so please report suspected
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
- whether symlinks, hard links, unusual file types, external commands, Project
  mode, or an LLM endpoint are involved; and
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

- Catomic must not make silent network calls or silently apply LLM output.
- LLM requests must name and confirm their endpoint and context before sending;
  proposed edits remain preview-only until separately confirmed.
- Project discovery and repository-aware tooling are opt-in.
- Commands and hooks in the user's configuration are trusted local code and run
  through `/bin/sh -c`; arbitrary side effects from a command the user configured
  are not a sandbox escape.
- Availability of a configured local or remote service is outside Catomic's
  security boundary, but sending data to a different or unconfirmed endpoint is
  in scope.
