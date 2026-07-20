//! Purpose: reject configuration keys that no current schema consumes.
//! Owns: structural key validation and full TOML paths for unknown entries.
//! Must not: apply settings, construct services, read credentials, or mutate files.
//! Invariants: dynamic table names remain open; fields inside each entry stay explicit.

use std::io;

use toml::{Table, Value};

const ROOT_KEYS: &[&str] = &[
    "big_files",
    "cat",
    "commands",
    "editor",
    "files",
    "hooks",
    "keybindings",
    "languages",
    "linters",
    "llm",
    "mobile",
    "recovery",
    "theme",
    "view",
];
const EDITOR_KEYS: &[&str] = &["tab_size"];
const BIG_FILE_KEYS: &[&str] = &["page_lines"];
const FILE_KEYS: &[&str] = &["auto_reload"];
const VIEW_KEYS: &[&str] = &["external_diff", "line_numbers"];
const CAT_KEYS: &[&str] = &["status_messages"];
const RECOVERY_KEYS: &[&str] = &["enabled", "interval_secs", "max_bytes"];
const MOBILE_KEYS: &[&str] = &["action_bar"];
const HOOK_KEYS: &[&str] = &["on_open", "on_save", "before_llm"];
const LANGUAGE_KEYS: &[&str] = &["tab_size", "linter", "llm"];
const LANGUAGE_LLM_KEYS: &[&str] = &["inline"];
const COMMAND_KEYS: &[&str] = &["command", "input", "output", "timeout_secs"];
const LLM_KEYS: &[&str] = &[
    "default",
    "base_url",
    "model",
    "api_key_env",
    "timeout_secs",
    "inline",
    "backends",
];
const INLINE_KEYS: &[&str] = &[
    "instruction_prefix",
    "instruction_suffix",
    "context_open",
    "context_close",
    "warn_lines",
    "block_mode",
    "queue_limit",
    "stop_on_error",
    "remove_instruction_after_apply",
];
const HTTP_BACKEND_KEYS: &[&str] = &[
    "type",
    "name",
    "model",
    "base_url",
    "api_key_env",
    "headers",
    "header_envs",
    "models",
    "discovery",
    "timeout_secs",
    "enabled",
];
const COMMAND_BACKEND_KEYS: &[&str] = &[
    "type",
    "name",
    "model",
    "program",
    "args",
    "input",
    "output",
    "timeout_secs",
    "enabled",
];
const BACKEND_KEYS: &[&str] = &[
    "type",
    "name",
    "model",
    "base_url",
    "api_key_env",
    "headers",
    "header_envs",
    "models",
    "discovery",
    "program",
    "args",
    "input",
    "output",
    "timeout_secs",
    "enabled",
];
const THEME_KEYS: &[&str] = &["name", "colors"];
const THEME_COLOR_KEYS: &[&str] = &[
    "text",
    "background",
    "cursor",
    "selection",
    "line_number",
    "status",
    "status_filename",
    "message",
    "status_warning",
    "status_prompt",
    "error",
    "markdown_heading",
    "markdown_emphasis",
    "markdown_code",
    "markdown_marker",
    "markdown_link",
    "syntax_keyword",
    "syntax_string",
    "syntax_comment",
    "syntax_number",
    "search_match",
    "diff_added",
    "diff_removed",
    "external_added",
    "external_changed",
    "external_deleted",
    "llm_changed",
    "preview",
];
const STYLE_KEYS: &[&str] = &["fg", "bg", "bold", "dim", "underline", "reverse"];

pub(crate) fn validate_unknown_keys(text: &str) -> io::Result<()> {
    let root = super::decode::<Table>(text)?;
    reject_unknown(&root, "", ROOT_KEYS)?;
    for (section, keys) in [
        ("editor", EDITOR_KEYS),
        ("big_files", BIG_FILE_KEYS),
        ("files", FILE_KEYS),
        ("view", VIEW_KEYS),
        ("cat", CAT_KEYS),
        ("recovery", RECOVERY_KEYS),
        ("mobile", MOBILE_KEYS),
        ("hooks", HOOK_KEYS),
    ] {
        validate_section(&root, section, keys)?;
    }
    validate_languages(&root)?;
    validate_commands(&root)?;
    validate_llm(&root)?;
    validate_theme(&root)?;
    Ok(())
}

fn validate_section(root: &Table, name: &str, allowed: &[&str]) -> io::Result<()> {
    if let Some(table) = root.get(name).and_then(Value::as_table) {
        reject_unknown(table, name, allowed)?;
    }
    Ok(())
}

fn validate_languages(root: &Table) -> io::Result<()> {
    let Some(languages) = root.get("languages").and_then(Value::as_table) else {
        return Ok(());
    };
    for (name, value) in languages {
        let Some(language) = value.as_table() else {
            continue;
        };
        let path = dynamic_path("languages", name);
        reject_unknown(language, &path, LANGUAGE_KEYS)?;
        let Some(llm) = language.get("llm").and_then(Value::as_table) else {
            continue;
        };
        let llm_path = format!("{path}.llm");
        reject_unknown(llm, &llm_path, LANGUAGE_LLM_KEYS)?;
        if let Some(inline) = llm.get("inline").and_then(Value::as_table) {
            validate_inline(inline, &format!("{llm_path}.inline"))?;
        }
    }
    Ok(())
}

fn validate_commands(root: &Table) -> io::Result<()> {
    let Some(commands) = root.get("commands").and_then(Value::as_table) else {
        return Ok(());
    };
    for (name, value) in commands {
        if let Some(command) = value.as_table() {
            reject_unknown(command, &dynamic_path("commands", name), COMMAND_KEYS)?;
        }
    }
    Ok(())
}

fn validate_llm(root: &Table) -> io::Result<()> {
    let Some(llm) = root.get("llm").and_then(Value::as_table) else {
        return Ok(());
    };
    reject_unknown(llm, "llm", LLM_KEYS)?;
    if let Some(inline) = llm.get("inline").and_then(Value::as_table) {
        validate_inline(inline, "llm.inline")?;
    }
    let Some(backends) = llm.get("backends").and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, value) in backends.iter().enumerate() {
        let Some(backend) = value.as_table() else {
            continue;
        };
        let allowed = match backend.get("type").and_then(Value::as_str) {
            Some("openai-compatible") => HTTP_BACKEND_KEYS,
            Some("command") => COMMAND_BACKEND_KEYS,
            _ => BACKEND_KEYS,
        };
        reject_unknown(backend, &format!("llm.backends[{index}]"), allowed)?;
    }
    Ok(())
}

fn validate_inline(table: &Table, path: &str) -> io::Result<()> {
    reject_unknown(table, path, INLINE_KEYS)
}

fn validate_theme(root: &Table) -> io::Result<()> {
    let Some(theme) = root.get("theme").and_then(Value::as_table) else {
        return Ok(());
    };
    reject_unknown(theme, "theme", THEME_KEYS)?;
    let Some(colors) = theme.get("colors").and_then(Value::as_table) else {
        return Ok(());
    };
    reject_unknown(colors, "theme.colors", THEME_COLOR_KEYS)?;
    for (role, value) in colors {
        if matches!(role.as_str(), "background" | "cursor") {
            continue;
        }
        if let Some(style) = value.as_table() {
            reject_unknown(style, &format!("theme.colors.{role}"), STYLE_KEYS)?;
        }
    }
    Ok(())
}

fn reject_unknown(table: &Table, path: &str, allowed: &[&str]) -> io::Result<()> {
    let Some(key) = table.keys().find(|key| !allowed.contains(&key.as_str())) else {
        return Ok(());
    };
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unknown configuration key {}", child_path(path, key)),
    ))
}

fn child_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn dynamic_path(parent: &str, key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if bare {
        format!("{parent}.{key}")
    } else {
        format!("{parent}.{key:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_unknown_keys_with_full_paths() {
        for (text, path) in [
            ("[edtor]\ntab_size = 2\n", "edtor"),
            ("[autocomplete]\nenabled = true\n", "autocomplete"),
            ("[editor]\ntab_szie = 2\n", "editor.tab_szie"),
            ("[files]\nauto_relod = false\n", "files.auto_relod"),
            (
                "[commands.format]\ncommand = \"rustfmt\"\ntimeot_secs = 3\n",
                "commands.format.timeot_secs",
            ),
            ("[languages.rs]\ntab_szie = 4\n", "languages.rs.tab_szie"),
            (
                "[[llm.backends]]\nname = \"local\"\ntype = \"command\"\nprogram = \"codex\"\nmodel = \"codex\"\noutput = \"codex-jsonl-v1\"\ntimeot_secs = 30\n",
                "llm.backends[0].timeot_secs",
            ),
            ("[theme.colors]\nstatuz = \"red\"\n", "theme.colors.statuz"),
            (
                "[theme.colors]\nautocomplete = \"bright-black\"\n",
                "theme.colors.autocomplete",
            ),
            (
                "[theme.colors]\nstatus = { fg = \"red\", blod = true }\n",
                "theme.colors.status.blod",
            ),
        ] {
            let error = validate_unknown_keys(text).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(
                error.to_string().contains(path),
                "{path:?} missing from {error}"
            );
        }
    }

    #[test]
    fn accepts_intentionally_dynamic_table_names() {
        let text = r#"
[languages."c++"]
tab_size = 2
linter = "clang-tidy {file}"

[languages."c++".llm.inline]
warn_lines = 20

[commands."format-c++"]
command = "clang-format"
timeout_secs = 5

[keybindings]
save = ["ctrl+s"]
"alt+x" = "quit"

[linters]
"c++" = "clang-tidy {file}"

[[llm.backends]]
name = "hosted"
type = "openai-compatible"
base_url = "https://models.example/v1"
model = "provider/model"
headers = { "X-Client" = "catomic" }
header_envs = { "X-Key" = "MODEL_KEY" }
"#;
        validate_unknown_keys(text).unwrap();
    }
}
