//! Purpose: prove preset parsing, compatibility translation, and bounded validation.
//! Owns: deterministic TOML fixtures for HTTP, command, static-model, and error cases.
//! Must not: read user config, inspect environment values, spawn commands, or network.
//! Invariants: old `[llm]` input produces the same effective local HTTP backend.

use super::*;

#[test]
fn missing_configuration_defaults_to_a_legacy_loopback_preset() {
    let catalog = LlmCatalog::default();
    let preset = catalog.default_preset();
    assert_eq!(catalog.default, "local");
    assert_eq!(preset.model, "local-model");
    let BackendAdapter::OpenAiCompatible(http) = &preset.adapter else {
        panic!("default must be HTTP")
    };
    assert_eq!(http.base_url, "http://127.0.0.1:8080/v1");
    assert_eq!(http.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
}

#[test]
fn translates_existing_single_llm_configuration() {
    let catalog = parse(
        "[other]\nmodel = \"ignored\"\n[llm]\nbase_url = \"HTTPS://Models.Example:443/v1/\"\n\
         model = \"cat-coder\"\napi_key_env = \"CATOMIC_TOKEN\"\ntimeout_secs = 30\n",
    )
    .unwrap();
    let preset = catalog.default_preset();
    assert_eq!(preset.name, "local");
    assert_eq!(preset.model, "cat-coder");
    let BackendAdapter::OpenAiCompatible(http) = &preset.adapter else {
        panic!("legacy must be HTTP")
    };
    assert_eq!(http.base_url, "https://models.example/v1");
    assert_eq!(http.api_key_env.as_deref(), Some("CATOMIC_TOKEN"));
    assert_eq!(http.timeout, Duration::from_secs(30));
}

#[test]
fn parses_named_http_and_two_structured_command_formats() {
    let catalog = parse(
        r#"
[llm]
default = "router"

[[llm.backends]]
name = "local llama"
type = "openai-compatible"
base_url = "http://127.0.0.1:11434/v1"
model = "llama"
models = ["small", "large", "small"]
discovery = true
headers = { "X-Title" = "Catomic" }

[[llm.backends]]
name = "router"
type = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
model = "openai/gpt"
api_key_env = "OPENROUTER_API_KEY"
header_envs = { "X-Provider-Key" = "PROVIDER_KEY" }

[[llm.backends]]
name = "claude"
type = "command"
program = "claude"
args = ["-p", "--output-format", "json"]
model = "sonnet"
output = "claude-json-v1"

[[llm.backends]]
name = "codex"
type = "command"
program = "/opt/codex bin/codex"
args = ["exec", "--json", "--model", "gpt-codex"]
model = "gpt-codex"
output = "codex-jsonl-v1"
enabled = false
"#,
    )
    .unwrap();

    assert_eq!(catalog.default_preset().name, "router");
    let local = catalog.find("local llama").unwrap();
    let BackendAdapter::OpenAiCompatible(http) = &local.adapter else {
        panic!("expected HTTP")
    };
    assert_eq!(http.models, ["small", "large"]);
    assert!(http.discovery);
    let codex = catalog.find("codex").unwrap();
    assert!(!codex.enabled);
    let BackendAdapter::Command(command) = &codex.adapter else {
        panic!("expected command")
    };
    assert_eq!(command.output, CommandOutputFormat::CodexJsonlV1);
    assert_eq!(command.args[3], "gpt-codex");
}

#[test]
fn rejects_ambiguous_duplicate_or_unknown_defaults() {
    for text in [
        "[llm]\nbase_url='http://localhost/v1'\n[[llm.backends]]\nname='x'\ntype='command'\nmodel='x'\nprogram='x'\noutput='claude-json-v1'\n",
        "[llm]\ndefault='missing'\n[[llm.backends]]\nname='x'\ntype='command'\nmodel='x'\nprogram='x'\noutput='claude-json-v1'\n",
        "[[llm.backends]]\nname='same'\ntype='command'\nmodel='a'\nprogram='a'\noutput='claude-json-v1'\n[[llm.backends]]\nname='same'\ntype='command'\nmodel='b'\nprogram='b'\noutput='codex-jsonl-v1'\n",
    ] {
        assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn rejects_unsafe_urls_env_names_headers_commands_and_bounds() {
    for text in [
        "[llm]\nbase_url = 'https://key@example.test/v1'\n",
        "[llm]\napi_key_env = 'bad-name'\n",
        "[llm]\ntimeout_secs = 0\n",
        "[[llm.backends]]\nname='x'\ntype='openai-compatible'\nmodel='x'\nbase_url='https://x/v1?key=x'\n",
        "[[llm.backends]]\nname='x'\ntype='openai-compatible'\nmodel='x'\nbase_url='https://x/v1'\nheaders={ 'bad header'='x' }\n",
        "[[llm.backends]]\nname='x'\ntype='openai-compatible'\nmodel='x'\nbase_url='https://x/v1'\nheaders={ 'X-Key'='a' }\nheader_envs={ 'x-key'='TOKEN' }\n",
        "[[llm.backends]]\nname='x'\ntype='openai-compatible'\nmodel='x'\nbase_url='https://x/v1'\napi_key_env='TOKEN'\nheaders={ authorization='Bearer fixed' }\n",
        "[[llm.backends]]\nname='x'\ntype='openai-compatible'\nmodel='x'\nbase_url='https://x/v1'\nheaders={ 'X-Api-Key'='fixed-secret' }\n",
        "[[llm.backends]]\nname='x'\ntype='command'\nmodel='x'\nprogram='./relative/tool'\noutput='claude-json-v1'\n",
        "[[llm.backends]]\nname='x'\ntype='command'\nmodel='x'\nprogram='tool'\noutput='unversioned'\n",
    ] {
        assert_eq!(parse(text).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    let oversized_header = format!(
        "[[llm.backends]]\nname='x'\ntype='openai-compatible'\nmodel='x'\nbase_url='https://x/v1'\nheaders={{ 'X-Metadata'={:?} }}\n",
        "x".repeat(8_193)
    );
    assert_eq!(
        parse(&oversized_header).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn malformed_toml_error_does_not_echo_source_values() {
    let error = parse("[llm]\ndefault = 'CATOMIC_SECRET_WITHOUT_END\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("source text suppressed"));
    assert!(!error.contains("CATOMIC_SECRET_WITHOUT_END"));
}

#[test]
fn parses_inline_defaults_and_language_overrides_with_named_backends() {
    let catalog = parse(
        r#"
[llm]
default = "local"

[llm.inline]
warn_lines = 700
block_mode = "queued"
queue_limit = 8
remove_instruction_after_apply = false

[[llm.backends]]
name = "local"
type = "openai-compatible"
base_url = "http://127.0.0.1:11434/v1"
model = "llama"

[languages.rs.llm.inline]
instruction_prefix = "// >>"

[languages.html.llm.inline]
instruction_prefix = "<!-- >>"
instruction_suffix = "-->"
"#,
    )
    .unwrap();

    assert_eq!(catalog.inline.warn_lines, 700);
    assert_eq!(catalog.inline.block_mode, InlineBlockMode::Queued);
    assert_eq!(catalog.inline.queue_limit, 8);
    assert!(!catalog.inline.remove_instruction_after_apply);
    let rust = catalog.inline_for_path(Some(Path::new("main.RS"))).unwrap();
    assert_eq!(rust.instruction_prefix, "// >>");
    assert_eq!(rust.context_open, "<catblock>");
    let html = catalog
        .inline_for_path(Some(Path::new("index.html")))
        .unwrap();
    assert_eq!(html.instruction_prefix, "<!-- >>");
    assert_eq!(html.instruction_suffix, "-->");
}

#[test]
fn rejects_unsafe_inline_markers_and_unbounded_limits() {
    for text in [
        "[llm.inline]\ninstruction_prefix = \"\"\n",
        "[llm.inline]\ncontext_open = \"same\"\ncontext_close = \"same\"\n",
        "[llm.inline]\ninstruction_prefix = \"<cat\"\ncontext_open = \"<catblock>\"\n",
        "[llm.inline]\ninstruction_suffix = \"<catblock>\"\n",
        "[llm.inline]\ncontext_close = \" bad\"\n",
        "[llm.inline]\nwarn_lines = 0\n",
        "[llm.inline]\nwarn_lines = 2001\n",
        "[llm.inline]\nqueue_limit = 0\n",
        "[llm.inline]\nqueue_limit = 65\n",
        "[llm.inline]\nblock_mode = \"parallel\"\n",
    ] {
        let error = parse(text).expect_err("invalid inline settings must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{text}");
    }
}
