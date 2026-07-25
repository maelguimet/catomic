//! Purpose: reject unknown configuration keys while accepting narrow retired compatibility input.
//! Owns: structural key validation and full TOML paths for unknown entries.
//! Must not: apply settings, construct services, read credentials, or mutate files.
//! Invariants: dynamic table names remain open; active and retired fields stay explicit.

use std::io;

use toml::{Table, Value};

const ROOT_KEYS: &[&str] = &[
    // Retained as inert input for configurations generated before autocomplete was removed.
    "autocomplete",
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
const RETIRED_AUTOCOMPLETE_KEYS: &[&str] = &[
    "enabled",
    "idle_debounce_ms",
    "minimum_prefix_length",
    "max_context_before",
    "max_context_after",
    "max_generated_tokens",
    "model",
    "allow_remote",
];
const MOBILE_KEYS: &[&str] = &["action_bar"];
const HOOK_KEYS: &[&str] = &["on_open", "on_save", "before_llm"];
const LANGUAGE_KEYS: &[&str] = &["tab_size", "linter", "llm"];
// Retained as inert input for configurations generated with inline clanker support.
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
// Retired inline fields remain structurally explicit so typos do not become silent config.
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
    // Retired picker inputs remain structurally explicit and inert.
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
    "lint",
    // Retained as an inert role for configurations generated with inline clanker support.
    "llm_changed",
    // Retained as an inert role for configurations generated before autocomplete was removed.
    "autocomplete",
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
    ] {
        validate_section(&root, section, keys)?;
    }
    validate_hooks(&root)?;
    validate_retired_autocomplete(&root)?;
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

fn validate_hooks(root: &Table) -> io::Result<()> {
    let Some(hooks) = optional_table(root, "hooks", "hooks")? else {
        return Ok(());
    };
    reject_unknown(hooks, "hooks", HOOK_KEYS)?;
    if let Some(value) = hooks.get("before_llm") {
        validate_string_array(value, "hooks.before_llm")?;
    }
    Ok(())
}

fn validate_retired_autocomplete(root: &Table) -> io::Result<()> {
    let Some(value) = root.get("autocomplete") else {
        return Ok(());
    };
    let Some(table) = value.as_table() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "configuration key autocomplete must be a table",
        ));
    };
    reject_unknown(table, "autocomplete", RETIRED_AUTOCOMPLETE_KEYS)
}

fn validate_languages(root: &Table) -> io::Result<()> {
    let Some(value) = root.get("languages") else {
        return Ok(());
    };
    let languages = value
        .as_table()
        .ok_or_else(|| invalid("configuration key languages must be a table"))?;
    for (name, value) in languages {
        let path = dynamic_path("languages", name);
        let language = value
            .as_table()
            .ok_or_else(|| invalid(format!("configuration key {path} must be a table")))?;
        reject_unknown(language, &path, LANGUAGE_KEYS)?;
        let llm_path = format!("{path}.llm");
        let Some(llm) = optional_table(language, "llm", &llm_path)? else {
            continue;
        };
        reject_unknown(llm, &llm_path, LANGUAGE_LLM_KEYS)?;
        if let Some(inline) = optional_table(llm, "inline", &format!("{llm_path}.inline"))? {
            validate_retired_inline(inline, &format!("{llm_path}.inline"))?;
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
    let Some(llm) = optional_table(root, "llm", "llm")? else {
        return Ok(());
    };
    reject_unknown(llm, "llm", LLM_KEYS)?;
    validate_optional_string(llm, "default", "llm.default")?;
    validate_optional_string(llm, "base_url", "llm.base_url")?;
    validate_optional_string(llm, "model", "llm.model")?;
    validate_optional_string(llm, "api_key_env", "llm.api_key_env")?;
    validate_optional_integer(llm, "timeout_secs", "llm.timeout_secs")?;
    if let Some(inline) = optional_table(llm, "inline", "llm.inline")? {
        validate_retired_inline(inline, "llm.inline")?;
    }
    let Some(value) = llm.get("backends") else {
        return Ok(());
    };
    let backends = value
        .as_array()
        .ok_or_else(|| invalid("configuration key llm.backends must be an array"))?;
    if backends.len() > 128 {
        return Err(invalid(
            "configuration key llm.backends exceeds 128 entries",
        ));
    }
    for (index, value) in backends.iter().enumerate() {
        let path = format!("llm.backends[{index}]");
        let backend = value
            .as_table()
            .ok_or_else(|| invalid(format!("configuration key {path} must be a table")))?;
        let allowed = match backend.get("type").and_then(Value::as_str) {
            Some("openai-compatible") => HTTP_BACKEND_KEYS,
            Some("command") => COMMAND_BACKEND_KEYS,
            _ => BACKEND_KEYS,
        };
        reject_unknown(backend, &path, allowed)?;
        validate_retired_backend(backend, &path)?;
    }
    Ok(())
}

fn validate_retired_inline(table: &Table, path: &str) -> io::Result<()> {
    reject_unknown(table, path, INLINE_KEYS)?;
    for key in [
        "instruction_prefix",
        "instruction_suffix",
        "context_open",
        "context_close",
        "block_mode",
    ] {
        validate_optional_string(table, key, &child_path(path, key))?;
    }
    for key in ["warn_lines", "queue_limit"] {
        validate_optional_integer(table, key, &child_path(path, key))?;
    }
    for key in ["stop_on_error", "remove_instruction_after_apply"] {
        validate_optional_boolean(table, key, &child_path(path, key))?;
    }
    Ok(())
}

fn validate_retired_backend(backend: &Table, path: &str) -> io::Result<()> {
    for key in [
        "type",
        "name",
        "model",
        "base_url",
        "api_key_env",
        "program",
        "input",
        "output",
    ] {
        validate_optional_string(backend, key, &child_path(path, key))?;
    }
    validate_optional_integer(backend, "timeout_secs", &child_path(path, "timeout_secs"))?;
    for key in ["discovery", "enabled"] {
        validate_optional_boolean(backend, key, &child_path(path, key))?;
    }
    if let Some(value) = backend.get("models") {
        validate_retired_models(value, &child_path(path, "models"))?;
    }
    if let Some(value) = backend.get("args") {
        validate_string_array(value, &child_path(path, "args"))?;
    }
    for key in ["headers", "header_envs"] {
        if let Some(value) = backend.get(key) {
            validate_string_table(value, &child_path(path, key))?;
        }
    }
    Ok(())
}

fn validate_retired_models(value: &Value, path: &str) -> io::Result<()> {
    let models = value
        .as_array()
        .ok_or_else(|| invalid(format!("configuration key {path} must be an array")))?;
    if models.len() > 128 {
        return Err(invalid(format!(
            "configuration key {path} exceeds 128 entries"
        )));
    }
    for (index, value) in models.iter().enumerate() {
        let model = value.as_str().ok_or_else(|| {
            invalid(format!(
                "configuration key {path}[{index}] must be a string"
            ))
        })?;
        let model = model.trim();
        if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
            return Err(invalid(format!(
                "configuration key {path}[{index}] must be 1-256 printable bytes"
            )));
        }
    }
    Ok(())
}

fn validate_optional_string(table: &Table, key: &str, path: &str) -> io::Result<()> {
    if table.get(key).is_some_and(|value| !value.is_str()) {
        return Err(invalid(format!(
            "configuration key {path} must be a string"
        )));
    }
    Ok(())
}

fn validate_optional_integer(table: &Table, key: &str, path: &str) -> io::Result<()> {
    if table.get(key).is_some_and(|value| !value.is_integer()) {
        return Err(invalid(format!(
            "configuration key {path} must be an integer"
        )));
    }
    Ok(())
}

fn validate_optional_boolean(table: &Table, key: &str, path: &str) -> io::Result<()> {
    if table.get(key).is_some_and(|value| !value.is_bool()) {
        return Err(invalid(format!(
            "configuration key {path} must be a boolean"
        )));
    }
    Ok(())
}

fn validate_string_array(value: &Value, path: &str) -> io::Result<()> {
    let values = value
        .as_array()
        .ok_or_else(|| invalid(format!("configuration key {path} must be an array")))?;
    for (index, value) in values.iter().enumerate() {
        if !value.is_str() {
            return Err(invalid(format!(
                "configuration key {path}[{index}] must be a string"
            )));
        }
    }
    Ok(())
}

fn validate_string_table(value: &Value, path: &str) -> io::Result<()> {
    let values = value
        .as_table()
        .ok_or_else(|| invalid(format!("configuration key {path} must be a table")))?;
    for (key, value) in values {
        if !value.is_str() {
            return Err(invalid(format!(
                "configuration key {} must be a string",
                dynamic_path(path, key)
            )));
        }
    }
    Ok(())
}

fn optional_table<'a>(parent: &'a Table, key: &str, path: &str) -> io::Result<Option<&'a Table>> {
    let Some(value) = parent.get(key) else {
        return Ok(None);
    };
    value.as_table().map(Some).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("configuration key {path} must be a table"),
        )
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
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
            (
                "[autocomplete]\nenabeld = true\n",
                "autocomplete.enabeld",
            ),
            ("autocomplete = true\n", "autocomplete must be a table"),
            ("[editor]\ntab_szie = 2\n", "editor.tab_szie"),
            ("[files]\nauto_relod = false\n", "files.auto_relod"),
            ("hooks = true\n", "hooks must be a table"),
            (
                "[hooks]\nbefore_llm = \"guard\"\n",
                "hooks.before_llm must be an array",
            ),
            (
                "[hooks]\nbefore_llm = [\"guard\", 1]\n",
                "hooks.before_llm[1] must be a string",
            ),
            (
                "[hooks]\nbefore-llm = []\n",
                "unknown configuration key hooks.before-llm",
            ),
            (
                "[commands.format]\ncommand = \"rustfmt\"\ntimeot_secs = 3\n",
                "commands.format.timeot_secs",
            ),
            ("[languages.rs]\ntab_szie = 4\n", "languages.rs.tab_szie"),
            (
                "[[llm.backends]]\nname = \"local\"\ntype = \"command\"\nprogram = \"codex\"\nmodel = \"codex\"\noutput = \"codex-jsonl-v1\"\ntimeot_secs = 30\n",
                "llm.backends[0].timeot_secs",
            ),
            ("llm = true\n", "llm must be a table"),
            ("[llm]\ndefault = 1\n", "llm.default must be a string"),
            (
                "[llm]\ntimeout_secs = \"slow\"\n",
                "llm.timeout_secs must be an integer",
            ),
            (
                "[llm]\nbackends = {}\n",
                "llm.backends must be an array",
            ),
            (
                "[llm]\nbackends = [1]\n",
                "llm.backends[0] must be a table",
            ),
            (
                "[[llm.backends]]\ntype = true\n",
                "llm.backends[0].type must be a string",
            ),
            (
                "[[llm.backends]]\nargs = [\"ok\", 1]\n",
                "llm.backends[0].args[1] must be a string",
            ),
            (
                "[[llm.backends]]\nheaders = []\n",
                "llm.backends[0].headers must be a table",
            ),
            (
                "[[llm.backends]]\nheaders = { \"X Header\" = 1 }\n",
                "llm.backends[0].headers.\"X Header\" must be a string",
            ),
            (
                "[[llm.backends]]\nheader_envs = { token = false }\n",
                "llm.backends[0].header_envs.token must be a string",
            ),
            (
                "[[llm.backends]]\nenabled = \"yes\"\n",
                "llm.backends[0].enabled must be a boolean",
            ),
            (
                "[[llm.backends]]\ntype = \"openai-compatible\"\nprogram = \"tool\"\n",
                "unknown configuration key llm.backends[0].program",
            ),
            (
                "[[llm.backends]]\ntype = \"command\"\nbase_url = \"https://example.test\"\n",
                "unknown configuration key llm.backends[0].base_url",
            ),
            ("[theme.colors]\nstatuz = \"red\"\n", "theme.colors.statuz"),
            (
                "[theme.colors]\nstatus = { fg = \"red\", blod = true }\n",
                "theme.colors.status.blod",
            ),
            (
                "[theme.colors]\nautocomplete = { blod = true }\n",
                "theme.colors.autocomplete.blod",
            ),
            ("llm.inline = true\n", "llm.inline must be a table"),
            (
                "[llm.inline]\nwarn_lines = \"many\"\n",
                "llm.inline.warn_lines must be an integer",
            ),
            (
                "[llm.inline]\nstop_on_error = \"yes\"\n",
                "llm.inline.stop_on_error must be a boolean",
            ),
            (
                "[llm.inline]\ninstruction_prefx = \">>\"\n",
                "llm.inline.instruction_prefx",
            ),
            ("languages = []\n", "languages must be a table"),
            (
                "[languages]\nrs = true\n",
                "languages.rs must be a table",
            ),
            (
                "[languages.rs]\nllm = true\n",
                "languages.rs.llm must be a table",
            ),
            (
                "[languages.rs.llm]\ninline = true\n",
                "languages.rs.llm.inline must be a table",
            ),
            (
                "[languages.rs.llm.inline]\nblock_mode = true\n",
                "languages.rs.llm.inline.block_mode must be a string",
            ),
            (
                "[[llm.backends]]\ntype = \"openai-compatible\"\nmodels = { typo = true }\n",
                "llm.backends[0].models must be an array",
            ),
            (
                "[[llm.backends]]\ntype = \"openai-compatible\"\nmodels = [1]\n",
                "llm.backends[0].models[0] must be a string",
            ),
            (
                "[[llm.backends]]\ntype = \"openai-compatible\"\nmodels = [\"\"]\n",
                "llm.backends[0].models[0] must be 1-256 printable bytes",
            ),
            (
                "[[llm.backends]]\ntype = \"openai-compatible\"\ndiscovery = { typo = true }\n",
                "llm.backends[0].discovery must be a boolean",
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
    fn accepts_retired_generated_autocomplete_configuration_as_inert_input() {
        let text = r#"
[autocomplete]
enabled = true
idle_debounce_ms = 750
minimum_prefix_length = 20
max_context_before = 2_048
max_context_after = 512
max_generated_tokens = 64
model = "retired-model"
allow_remote = true

[theme.colors]
autocomplete = { fg = "bright-black", dim = true }
"#;

        crate::config::validate_text(text).unwrap();
        assert_eq!(
            crate::config::theme::parse(text).unwrap(),
            crate::config::theme::Theme::default()
        );
    }

    #[test]
    fn accepts_complete_retired_ai_configuration_as_inert_input() {
        let text = include_str!("../../tests/fixtures/retired_ai_config.toml");

        crate::config::validate_text(text).unwrap();
        assert_eq!(
            crate::config::commands::parse(text).unwrap(),
            crate::config::commands::CommandConfig::default()
        );
        assert_eq!(
            crate::config::theme::parse(text).unwrap(),
            crate::config::theme::Theme::default()
        );
    }

    #[test]
    fn retired_model_list_keeps_its_previous_bounds_and_content_checks() {
        let models = (0..129)
            .map(|index| format!("\"model-{index}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!("[[llm.backends]]\ntype = \"openai-compatible\"\nmodels = [{models}]\n");
        let error = validate_unknown_keys(&text).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("llm.backends[0].models exceeds 128 entries"),
            "{error}"
        );
        for invalid in ["[\"\"]", "[\"\\n\"]"] {
            let text =
                format!("[[llm.backends]]\ntype = \"openai-compatible\"\nmodels = {invalid}\n");
            let error = validate_unknown_keys(&text).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("llm.backends[0].models[0] must be 1-256 printable bytes"),
                "{error}"
            );
        }
    }

    #[test]
    fn retired_backends_keep_their_previous_catalog_bound() {
        let backends = std::iter::repeat_n("{}", 129)
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!("[llm]\nbackends = [{backends}]\n");
        let error = validate_unknown_keys(&text).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("llm.backends exceeds 128 entries"),
            "{error}"
        );
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
