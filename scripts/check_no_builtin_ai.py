#!/usr/bin/env python3
"""Reject built-in AI runtime, commands, networking, and generated UI residue.

The compatibility parser intentionally still accepts retired configuration shapes.
This gate therefore uses exact path, symbol, dependency, and generated-surface rules
instead of broad words such as "model", "diff", or "preview".
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


def repository_files(root: Path) -> Iterable[Path]:
    for base in (root / "src", root / "tests"):
        if not base.exists():
            continue
        yield from (path for path in base.rglob("*") if path.is_file())


def relative(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def is_update_rust(path: str) -> bool:
    return path == "src/update.rs" or path.startswith("src/update/")


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
    direct_packages = {
        specification.get("package", alias)
        if isinstance(specification, dict)
        else alias
        for table in dependency_tables
        for alias, specification in table.items()
    }
    for package in ("reqwest", "tokio"):
        has_disallowed_entry = any(
            (
                specification.get("package", alias)
                if isinstance(specification, dict)
                else alias
            )
            == package
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
    print("No built-in AI runtime, command, generated UI, or network residue found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
