#!/usr/bin/env python3
"""Tripwire for known built-in-AI residue and architectural ownership seams.

The compatibility parser intentionally still accepts retired configuration shapes.
This gate therefore uses exact path, symbol, dependency, and generated-surface rules
instead of broad words such as "model", "diff", or "preview". It is a
defense-in-depth regression check, not semantic proof that arbitrary new code is
AI-free.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


FORBIDDEN_FILES = (
    "src/app/inline_clanker.rs",
    "src/app/llm_preview.rs",
    "src/app/llm_request.rs",
    "src/app/model_picker.rs",
    "src/app/model_session.rs",
    "src/app/repo_llm.rs",
    "src/config/llm.rs",
    "src/llm.rs",
    "src/tests/golden_phase6.rs",
    "tests/pty_inline_clanker.rs",
)

FORBIDDEN_DIRECTORIES = (
    "src/app/inline_clanker",
    "src/app/llm_preview",
    "src/app/llm_request",
    "src/app/model_picker",
    "src/app/repo_llm",
    "src/config/llm",
    "src/llm",
)

RUNTIME_IDENTIFIER = re.compile(
    r"\b(?:"
    r"BackendAdapter|"
    r"BackendErrorKind|"
    r"BackendMessage|"
    r"BackendPreset|"
    r"BackendRunner|"
    r"BeforeLlm|"
    r"CachedModels|"
    r"ChatMessage|"
    r"ChatRequest|"
    r"ChatResponse|"
    r"CommandBackend|"
    r"CommandInputFormat|"
    r"CommandOutputFormat|"
    r"ConfirmedAdapter|"
    r"ConfirmedBackend|"
    r"ContextBroker|"
    r"CurrentLlmCommand|"
    r"DiscoveryLimits|"
    r"DiscoveryResult|"
    r"DiscoveryTask|"
    r"HttpBackend|"
    r"InlineBlockMode|"
    r"InlineClanker|"
    r"InlineClankerState|"
    r"InlineDraft|"
    r"InlineError|"
    r"InlineScope|"
    r"InlineSettings|"
    r"InstructionBlock|"
    r"InstructionMetadata|"
    r"InstructionParseError|"
    r"LlmCatalog|"
    r"LlmConfig|"
    r"LlmError|"
    r"LlmTask(?:Result)?|"
    r"MessageRole|"
    r"ModelListEntry|"
    r"ModelListResponse|"
    r"ModelPicker|"
    r"ModelPickerState|"
    r"ModelSession|"
    r"OpenAiCompatClient|"
    r"PendingLlmRequest|"
    r"PreparedRepoContext|"
    r"PreparedWorkflow|"
    r"RawBackend|"
    r"RawInlineSettings|"
    r"RawLanguageLlmSettings|"
    r"RawLlm|"
    r"RepoCheckResult|"
    r"RepoCheckTask|"
    r"RepoLlm|"
    r"RepoLlmCommand|"
    r"RepoLlmState|"
    r"RepoLlmTask(?:Result)?|"
    r"RepoPrepareResult|"
    r"RepoPrepareTask|"
    r"ResponseMessage|"
    r"RunnerAdapter|"
    r"RunningDiscovery|"
    r"RunningLlmRequest|"
    r"FULL_FILE_SYSTEM_PROMPT|"
    r"MULTI_SYSTEM_PROMPT|"
    r"REGION_SYSTEM_PROMPT|"
    r"SYSTEM_PROMPT|"
    r"before_current_llm|"
    r"begin_before_llm|"
    r"broker_protocol|"
    r"collect_meow|"
    r"command_adapter|"
    r"inline_clanker|"
    r"llm_preview|"
    r"llm_request|"
    r"model_work_active|"
    r"model_picker|"
    r"model_session|"
    r"openai_compat|"
    r"repo_context|"
    r"repo_llm|"
    r"repo_prepare|"
    r"repo_task"
    r")\b"
)

LLM_MODULE_REFERENCE = re.compile(
    r"\bmod\s+llm\s*;|"
    r"\b(?:crate|super|self)::llm::|"
    r"\bconfig::llm(?:\b|::)"
)

# The updater owns direct network access. Match the current runtime/client crates,
# obvious alternatives, and socket APIs without banning inert address types or
# trusted external commands.
NETWORK_API = re.compile(
    r"\b(?:attohttpc|awc|curl|hyper|hyper_util|isahc|reqwest|surf|tokio|ureq)"
    r"(?:::\w+|\b)|"
    r"\b(?:TcpListener|TcpStream|UdpSocket)\b"
)

PROCESS_API = re.compile(
    r"\b(?:::)?std\s*::\s*process\s*::\s*Command\s*::\s*new\b|"
    r"\btype\s+\w+\s*=\s*(?:::)?std\s*::\s*process\s*::\s*Command\b"
)

RUST_USE_STATEMENT = re.compile(r"\buse\b[^;]*;")
EXTERN_STD_ALIAS = re.compile(
    r"\bextern\s+crate\s+std\s+as\s+([A-Za-z_]\w*)\s*;"
)

PROCESS_OWNER_FILES = {
    "src/clipboard.rs",
    "src/external/open_link.rs",
    "src/external/task.rs",
}

FORBIDDEN_DIRECT_DEPENDENCIES = {
    "anthropic",
    "async-openai",
    "attohttpc",
    "awc",
    "curl",
    "genai",
    "hyper",
    "hyper-util",
    "isahc",
    "llm",
    "llm-chain",
    "ollama-rs",
    "openai",
    "reqwest-middleware",
    "rig-core",
    "surf",
    "ureq",
}

LEGACY_PROMPT = re.compile(
    r"(?i)(?<![A-Za-z0-9_])"
    r"(?:bigmeow|feralmeow|gitmeow|megameow|meow)"
    r"(?![A-Za-z0-9_])"
)

RETIRED_ACTION = re.compile(
    r"(?i)(?:"
    r"clear-clanker-changes|"
    r"inline-meow|"
    r"picker-accept|"
    r"picker-cancel|"
    r"run-clanker|"
    r"select-model"
    r")"
)

RETIRED_CONFIG_TOKEN = re.compile(
    r"\b(?:api_key_env|before_llm|header_envs|llm_changed)\b|"
    r"openai-compatible"
)

LEGACY_PROMPT_ALLOWED = {
    "src/app/command_prompt/tests.rs",
    "src/app/help/tests.rs",
    "src/config/keybindings/tests.rs",
    "src/help_catalog/tests.rs",
    "tests/pty_smoke.rs",
}

RETIRED_ACTION_ALLOWED = {
    "src/config/keybindings.rs",
    "src/config/keybindings/tests.rs",
    "src/config/validation.rs",
    "src/help_catalog/tests.rs",
    "tests/fixtures/retired_ai_config.toml",
}

RETIRED_CONFIG_ALLOWED = {
    "src/app/help/tests.rs",
    "src/config/commands.rs",
    "src/config/theme/tests.rs",
    "src/config/validation.rs",
    "tests/fixtures/retired_ai_config.toml",
    "tests/pty_smoke.rs",
}

GENERATED_SURFACES = {
    "src/config/config_template.toml": "generated configuration",
    "src/app/help.rs": "built-in help",
    "src/help_catalog/prompt_commands.rs": "prompt catalog",
}

GENERATED_AI_TERM = re.compile(
    r"(?i)"
    r"\b(?:ai|anthropic|bigmeow|clanker|feralmeow|gitmeow|llm|"
    r"megameow|meow|models?|openai)\b|"
    r"(?:api_key_env|before_llm|header_envs|llm_changed)|"
    r"(?:clear-clanker-changes|inline-meow|picker-accept|picker-cancel|"
    r"run-clanker|select-model)"
)


@dataclass(frozen=True, order=True)
class Violation:
    path: str
    line: int
    rule: str
    detail: str

    def render(self) -> str:
        location = self.path if self.line == 0 else f"{self.path}:{self.line}"
        return f"{location}: {self.rule}: {self.detail}"


def line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def rust_char_literal_end(text: str, index: int) -> int | None:
    """Return the end of a Rust char/byte-char literal, but not a lifetime."""

    quote = index + 1 if text.startswith("b'", index) else index
    if quote >= len(text) or text[quote] != "'":
        return None
    cursor = quote + 1
    if cursor >= len(text) or text[cursor] in {"\r", "\n", "'"}:
        return None

    if text[cursor] != "\\":
        cursor += 1
    else:
        cursor += 1
        if cursor >= len(text) or text[cursor] in {"\r", "\n"}:
            return None
        escape = text[cursor]
        cursor += 1
        if escape == "x":
            digits = text[cursor : cursor + 2]
            if len(digits) != 2 or any(
                character not in "0123456789abcdefABCDEF" for character in digits
            ):
                return None
            cursor += 2
        elif escape == "u":
            if cursor >= len(text) or text[cursor] != "{":
                return None
            close = text.find("}", cursor + 1)
            if close < 0 or "\n" in text[cursor:close]:
                return None
            digits = text[cursor + 1 : close].replace("_", "")
            if (
                not 1 <= len(digits) <= 6
                or any(character not in "0123456789abcdefABCDEF" for character in digits)
            ):
                return None
            cursor = close + 1

    if cursor >= len(text) or text[cursor] != "'":
        return None
    return cursor + 1


def rust_code_only(text: str) -> str:
    """Mask Rust comments and string contents while preserving byte positions/newlines."""

    output = list(text)
    index = 0
    length = len(text)
    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index + 2)
            end = length if end < 0 else end
            for cursor in range(index, end):
                output[cursor] = " "
            index = end
            continue
        if text.startswith("/*", index):
            depth = 1
            cursor = index + 2
            while cursor < length and depth:
                if text.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif text.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            for position in range(index, cursor):
                if output[position] != "\n":
                    output[position] = " "
            index = cursor
            continue

        char_end = rust_char_literal_end(text, index)
        if char_end is not None:
            for position in range(index, char_end):
                if output[position] != "\n":
                    output[position] = " "
            index = char_end
            continue

        raw = re.match(r"(?:br|r)(?P<hashes>#{0,255})\"", text[index:])
        if raw:
            hashes = raw.group("hashes")
            end_marker = '"' + hashes
            content_start = index + raw.end()
            end = text.find(end_marker, content_start)
            end = length if end < 0 else end + len(end_marker)
            for position in range(index, end):
                if output[position] != "\n":
                    output[position] = " "
            index = end
            continue

        quote_index = index + 1 if text.startswith('b"', index) else index
        if quote_index < length and text[quote_index] == '"':
            cursor = quote_index + 1
            while cursor < length:
                if text[cursor] == "\\":
                    cursor += 2
                elif text[cursor] == '"':
                    cursor += 1
                    break
                else:
                    cursor += 1
            for position in range(index, min(cursor, length)):
                if output[position] != "\n":
                    output[position] = " "
            index = cursor
            continue
        index += 1
    return "".join(output)


def rust_production_code_only(code: str) -> str:
    """Mask inline #[cfg(test)] modules in already lexical-masked Rust code."""

    output = list(code)
    module = re.compile(
        r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*"
        r"(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+[A-Za-z_]\w*\s*\{"
    )
    for match in module.finditer(code):
        open_brace = code.rfind("{", match.start(), match.end())
        depth = 0
        end = len(code)
        for cursor in range(open_brace, len(code)):
            if code[cursor] == "{":
                depth += 1
            elif code[cursor] == "}":
                depth -= 1
                if depth == 0:
                    end = cursor + 1
                    break
        for cursor in range(match.start(), end):
            if output[cursor] != "\n":
                output[cursor] = " "
    return "".join(output)


def rust_use_entries(statement: str) -> list[tuple[tuple[str, ...], str | None]]:
    """Expand enough of a Rust use tree to identify std::process ownership."""

    tokens = re.findall(r"::|[{},;*]|[A-Za-z_]\w*", statement)
    if not tokens or tokens[0] != "use":
        return []
    index = 1
    entries: list[tuple[tuple[str, ...], str | None]] = []

    def parse_group(prefix: tuple[str, ...]) -> None:
        nonlocal index
        if index >= len(tokens) or tokens[index] != "{":
            return
        index += 1
        while index < len(tokens) and tokens[index] != "}":
            if tokens[index] == ",":
                index += 1
                continue
            parse_tree(prefix)
        if index < len(tokens) and tokens[index] == "}":
            index += 1

    def parse_tree(prefix: tuple[str, ...]) -> None:
        nonlocal index
        while index < len(tokens) and tokens[index] == "::":
            index += 1
        if index >= len(tokens):
            return
        if tokens[index] == "{":
            parse_group(prefix)
            return

        segment = tokens[index]
        if not re.fullmatch(r"[A-Za-z_]\w*|\*", segment):
            index += 1
            return
        index += 1
        if segment == "self":
            path = prefix
        else:
            path = (*prefix, segment)

        if index < len(tokens) and tokens[index] == "as":
            index += 1
            alias = tokens[index] if index < len(tokens) else None
            if alias is not None:
                index += 1
            entries.append((path, alias))
            return
        if index < len(tokens) and tokens[index] == "::":
            index += 1
            if index < len(tokens) and tokens[index] == "{":
                parse_group(path)
            else:
                parse_tree(path)
            return
        entries.append((path, None))

    if index < len(tokens) and tokens[index] == "{":
        parse_group(())
    else:
        parse_tree(())
    return entries


def process_api_matches(code: str) -> list[re.Match[str]]:
    """Find direct process APIs outside the reviewed production owners."""

    normalized = re.sub(r"\br#(?=[A-Za-z_])", "  ", code)
    matches = list(PROCESS_API.finditer(normalized))
    use_entries = [
        (match, rust_use_entries(match.group()))
        for match in RUST_USE_STATEMENT.finditer(normalized)
    ]
    std_aliases = {
        alias
        for _, entries in use_entries
        for path, alias in entries
        if path == ("std",) and alias is not None
    }
    std_aliases.update(EXTERN_STD_ALIAS.findall(normalized))
    for match, entries in use_entries:
        if any(
            path[:2] == ("std", "process")
            or (
                len(path) >= 2
                and path[0] in std_aliases
                and path[1] == "process"
            )
            for path, _ in entries
        ):
            matches.append(match)
    for alias in std_aliases:
        escaped = re.escape(alias)
        matches.extend(
            re.finditer(
                rf"\b(?:{escaped}\s*::\s*process\s*::\s*Command\s*::\s*new|"
                rf"type\s+\w+\s*=\s*{escaped}\s*::\s*process\s*::\s*Command)\b",
                normalized,
            )
        )

    return sorted(
        {(match.start(), match.end(), match.group()): match for match in matches}.values(),
        key=lambda match: (match.start(), match.end()),
    )


def repository_files(root: Path) -> Iterable[Path]:
    for base in (root / "src", root / "tests"):
        if not base.exists():
            continue
        yield from (path for path in base.rglob("*") if path.is_file())


def relative(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def is_update_rust(path: str) -> bool:
    return path == "src/update.rs" or path.startswith("src/update/")


def is_test_rust(path: str) -> bool:
    return (
        path.startswith("tests/")
        or path.startswith("src/tests/")
        or "/tests/" in path
        or path.endswith("/tests.rs")
    )


def path_violations(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for name in FORBIDDEN_FILES:
        path = root / name
        if path.exists() or path.is_symlink():
            violations.append(Violation(name, 0, "deleted path", "file must stay removed"))
    for name in FORBIDDEN_DIRECTORIES:
        path = root / name
        contains_file = path.is_dir() and any(
            child.is_file() or child.is_symlink() for child in path.rglob("*")
        )
        if path.is_symlink() or contains_file or (path.exists() and not path.is_dir()):
            violations.append(
                Violation(name, 0, "deleted path", "path family must contain no files")
            )
    return violations


def rust_violations(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in repository_files(root):
        if path.suffix != ".rs":
            continue
        name = relative(path, root)
        text = read_text(path)
        code = rust_code_only(text)
        production_code = rust_production_code_only(code)
        for rule, pattern in (
            ("retired runtime symbol", RUNTIME_IDENTIFIER),
            ("retired llm module", LLM_MODULE_REFERENCE),
        ):
            for match in pattern.finditer(code):
                violations.append(
                    Violation(name, line_number(text, match.start()), rule, match.group())
                )
        if not is_update_rust(name):
            for match in NETWORK_API.finditer(code):
                violations.append(
                    Violation(
                        name,
                        line_number(text, match.start()),
                        "network API outside updater",
                        match.group(),
                    )
                )
        if (
            not is_update_rust(name)
            and not is_test_rust(name)
            and name not in PROCESS_OWNER_FILES
        ):
            for match in process_api_matches(production_code):
                violations.append(
                    Violation(
                        name,
                        line_number(text, match.start()),
                        "process API outside owner",
                        match.group(),
                    )
                )
    return violations


def compatibility_token_violations(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in repository_files(root):
        if path.suffix not in {".rs", ".toml"}:
            continue
        name = relative(path, root)
        text = read_text(path)
        for rule, pattern, allowed in (
            ("legacy prompt name", LEGACY_PROMPT, LEGACY_PROMPT_ALLOWED),
            ("retired action name", RETIRED_ACTION, RETIRED_ACTION_ALLOWED),
            (
                "retired configuration token",
                RETIRED_CONFIG_TOKEN,
                RETIRED_CONFIG_ALLOWED,
            ),
        ):
            if name in allowed:
                continue
            for match in pattern.finditer(text):
                violations.append(
                    Violation(name, line_number(text, match.start()), rule, match.group())
                )
    return violations


def generated_surface_violations(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for name, label in GENERATED_SURFACES.items():
        path = root / name
        if not path.is_file():
            violations.append(
                Violation(name, 0, "required generated surface", f"{label} is missing")
            )
            continue
        text = read_text(path)
        for match in GENERATED_AI_TERM.finditer(text):
            violations.append(
                Violation(
                    name,
                    line_number(text, match.start()),
                    f"AI term in {label}",
                    match.group(),
                )
            )
    return violations


def dependency_violations(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    manifest_path = root / "Cargo.toml"
    lock_path = root / "Cargo.lock"
    try:
        manifest = tomllib.loads(read_text(manifest_path))
    except (OSError, tomllib.TOMLDecodeError) as error:
        return [Violation("Cargo.toml", 0, "manifest policy", str(error))]

    dependencies = manifest.get("dependencies", {})
    if not isinstance(dependencies, dict):
        violations.append(
            Violation(
                "Cargo.toml",
                0,
                "manifest policy",
                "[dependencies] must be a table",
            )
        )
        dependencies = {}
    dependency_tables = [dependencies]
    targets = manifest.get("target", {})
    if isinstance(targets, dict):
        for target in targets.values():
            if isinstance(target, dict):
                dependency_tables.extend(
                    table
                    for key in ("dependencies", "dev-dependencies", "build-dependencies")
                    if isinstance((table := target.get(key)), dict)
                )
    for key in ("dev-dependencies", "build-dependencies"):
        table = manifest.get(key)
        if isinstance(table, dict):
            dependency_tables.append(table)

    workspace = manifest.get("workspace", {})
    workspace_dependencies = (
        workspace.get("dependencies", {}) if isinstance(workspace, dict) else {}
    )
    if not isinstance(workspace_dependencies, dict):
        workspace_dependencies = {}

    def resolved_package(alias: str, specification: object) -> str:
        if isinstance(specification, dict) and specification.get("workspace") is True:
            specification = workspace_dependencies.get(alias, specification)
        if isinstance(specification, dict):
            package = specification.get("package", alias)
            return package if isinstance(package, str) else alias
        return alias

    direct_packages = {
        resolved_package(alias, specification)
        for table in dependency_tables
        for alias, specification in table.items()
    }
    for package in ("reqwest", "tokio"):
        has_disallowed_entry = any(
            resolved_package(alias, specification) == package
            and (table is not dependencies or alias != package)
            for table in dependency_tables
            for alias, specification in table.items()
        )
        if has_disallowed_entry:
            violations.append(
                Violation(
                    "Cargo.toml",
                    0,
                    "manifest policy",
                    f"{package} must appear only as the exact root [dependencies].{package} entry",
                )
            )
    for name in sorted(FORBIDDEN_DIRECT_DEPENDENCIES.intersection(direct_packages)):
        violations.append(
            Violation(
                "Cargo.toml",
                0,
                "manifest policy",
                f"direct network or AI dependency is forbidden: {name}",
            )
        )

    reqwest = dependencies.get("reqwest")
    tokio = dependencies.get("tokio")
    if not isinstance(reqwest, dict):
        violations.append(
            Violation("Cargo.toml", 0, "manifest policy", "reqwest must use a feature table")
        )
    else:
        if reqwest.get("default-features") is not False:
            violations.append(
                Violation(
                    "Cargo.toml",
                    0,
                    "manifest policy",
                    "reqwest default-features must be false",
                )
            )
        features = reqwest.get("features")
        if not isinstance(features, list) or set(features) != {"rustls-tls"}:
            violations.append(
                Violation(
                    "Cargo.toml",
                    0,
                    "manifest policy",
                    "reqwest features must be exactly ['rustls-tls']",
                )
            )
    if not isinstance(tokio, dict):
        violations.append(
            Violation("Cargo.toml", 0, "manifest policy", "tokio must use a feature table")
        )
    else:
        if tokio.get("default-features") is not False:
            violations.append(
                Violation(
                    "Cargo.toml",
                    0,
                    "manifest policy",
                    "tokio default-features must be false",
                )
            )
        features = tokio.get("features")
        if not isinstance(features, list) or set(features) != {"rt"}:
            violations.append(
                Violation(
                    "Cargo.toml",
                    0,
                    "manifest policy",
                    "tokio features must be exactly ['rt']",
                )
            )

    try:
        lock = tomllib.loads(read_text(lock_path))
    except (OSError, tomllib.TOMLDecodeError) as error:
        violations.append(Violation("Cargo.lock", 0, "lock policy", str(error)))
        return violations
    packages = lock.get("package", [])
    if any(package.get("name") == "tokio-macros" for package in packages):
        violations.append(
            Violation("Cargo.lock", 0, "lock policy", "tokio-macros must be absent")
        )
    for package in packages:
        if package.get("name") != "tokio":
            continue
        dependencies = package.get("dependencies", [])
        if any(
            dependency == "tokio-macros"
            or (
                isinstance(dependency, str)
                and dependency.startswith("tokio-macros ")
            )
            for dependency in dependencies
        ):
            violations.append(
                Violation(
                    "Cargo.lock",
                    0,
                    "lock policy",
                    "tokio must not depend on tokio-macros",
                )
            )
    return violations


def check_repository(root: Path) -> list[Violation]:
    root = root.resolve()
    violations = [
        *path_violations(root),
        *rust_violations(root),
        *compatibility_token_violations(root),
        *generated_surface_violations(root),
        *dependency_violations(root),
    ]
    return sorted(set(violations))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root (defaults to this script's checkout)",
    )
    args = parser.parse_args(argv)
    violations = check_repository(args.root)
    if violations:
        print("Built-in AI residue gate failed:", file=sys.stderr)
        for violation in violations:
            print(f"  {violation.render()}", file=sys.stderr)
        return 1
    print(
        "No known built-in-AI residue or unexpected direct network/process API found."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
