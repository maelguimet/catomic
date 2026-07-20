#!/usr/bin/env python3
"""Purpose: reject broken local links and duplicate decision-record numbers.
Owns: tracked Markdown discovery, relative links, and decision-record numbering.
Must not: make network requests, validate remote URLs, or rewrite documentation.
Invariants: checked targets exist inside the repository; decision IDs are unique.
Phase: post-v0.1 documentation maintenance.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit


INLINE_LINK = re.compile(
    r"!?\[[^\]\n]*\]\(\s*(?P<target><[^>\n]+>|(?:\\.|[^\s)\n])+)",
)
REFERENCE_DEFINITION = re.compile(
    r"^\s{0,3}\[[^\]\n]+\]:\s*(?P<target><[^>\n]+>|\S+)",
)
FENCE = re.compile(r"^\s{0,3}(?P<marker>`{3,}|~{3,})")
INLINE_CODE = re.compile(r"(`+)(?:[^`]|`(?!\1))*\1")
DECISION_RECORD = re.compile(r"^(?P<number>\d{4})-.+\.md$")


def tracked_markdown(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.md"],
        cwd=root,
        check=True,
        capture_output=True,
    )
    return [root / path.decode() for path in result.stdout.split(b"\0") if path]


def markdown_targets(source: Path) -> list[tuple[int, str]]:
    targets: list[tuple[int, str]] = []
    fence_marker: str | None = None

    for line_number, line in enumerate(
        source.read_text(encoding="utf-8").splitlines(), start=1
    ):
        fence = FENCE.match(line)
        if fence:
            marker = fence.group("marker")
            if fence_marker is None:
                fence_marker = marker[0]
            elif marker[0] == fence_marker:
                fence_marker = None
            continue
        if fence_marker is not None:
            continue

        prose = INLINE_CODE.sub("", line)
        for match in INLINE_LINK.finditer(prose):
            targets.append((line_number, match.group("target")))
        reference = REFERENCE_DEFINITION.match(prose)
        if reference:
            targets.append((line_number, reference.group("target")))

    return targets


def local_path(target: str) -> str | None:
    if target.startswith("<") and target.endswith(">"):
        target = target[1:-1]
    target = target.replace(r"\ ", " ")
    parsed = urlsplit(target)
    if parsed.scheme or parsed.netloc or not parsed.path or parsed.path.startswith("/"):
        return None
    return unquote(parsed.path)


def broken_links(root: Path) -> list[str]:
    failures: list[str] = []
    for source in tracked_markdown(root):
        for line_number, target in markdown_targets(source):
            path = local_path(target)
            if path is None:
                continue
            resolved = (source.parent / path).resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                reason = "target escapes the repository"
            else:
                if resolved.exists():
                    continue
                reason = "target does not exist"
            relative_source = source.relative_to(root)
            failures.append(f"{relative_source}:{line_number}: {target} ({reason})")
    return failures


def duplicate_decision_numbers(root: Path) -> list[str]:
    records: dict[str, list[Path]] = {}
    decision_dir = root / "docs" / "decisions"
    for source in tracked_markdown(root):
        match = DECISION_RECORD.match(source.name)
        if source.parent != decision_dir or match is None:
            continue
        records.setdefault(match.group("number"), []).append(source)
    return [
        f"{number}: {', '.join(str(path.relative_to(root)) for path in paths)}"
        for number, paths in sorted(records.items())
        if len(paths) > 1
    ]


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    link_failures = broken_links(root)
    duplicate_numbers = duplicate_decision_numbers(root)
    if link_failures:
        print("Broken local Markdown links:", file=sys.stderr)
        for failure in link_failures:
            print(f"  {failure}", file=sys.stderr)
    if duplicate_numbers:
        print("Duplicate decision-record numbers:", file=sys.stderr)
        for duplicate in duplicate_numbers:
            print(f"  {duplicate}", file=sys.stderr)
    if link_failures or duplicate_numbers:
        return 1
    print(f"Checked local links in {len(tracked_markdown(root))} tracked Markdown files.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
