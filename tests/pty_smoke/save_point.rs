//! Purpose: reproduce and stress save-point ordering through the real PTY binary.
//! Owns: content-free trace assertions for rapid save/quit, paste, hooks, Save As,
//!   and duplicate-path aliases.
//! Must not: use fixed timing sleeps, log document contents, or contact external services.
//! Invariants: every successful traced save ends at one exact clean history position.
//! Phase: issue #135 intermittent save-point investigation.

use super::*;

use serde_json::Value;

fn spawn_traced(path: &PathBuf, config_root: &PathBuf, trace: &PathBuf) -> TestResult<PtyEditor> {
    let environment = TempProject::new("save_trace_environment");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.arg(path);
    command.env("CATOMIC_TEST_SAVE_TRACE", trace);
    command.env("XDG_CONFIG_HOME", config_root);
    command.env("XDG_STATE_HOME", &environment.root);
    command.env("HOME", &environment.root);
    command.env("TERM", "xterm-256color");
    PtyEditor::spawn_command_with_environment(command, environment)
}

fn read_trace(path: &PathBuf) -> TestResult<Vec<Value>> {
    fs::read_to_string(path)?
        .lines()
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

fn assert_clean_save_then_quit(trace: &[Value]) {
    let save_end = trace
        .iter()
        .position(|entry| entry["event"] == "save_end")
        .expect("trace must contain save_end");
    let saved = &trace[save_end];
    assert_eq!(saved["atomic_write_attempted"], true);
    assert_eq!(saved["atomic_write_succeeded"], true);
    assert_eq!(saved["state"]["dirty"], false);
    assert_eq!(
        saved["state"]["history_position"],
        saved["state"]["saved_history_position"]
    );
    assert!(
        !trace[save_end + 1..]
            .iter()
            .any(|entry| entry["event"] == "content_edit"),
        "no content mutation may be hidden between the successful save and quit"
    );
    let quit = trace
        .iter()
        .find(|entry| entry["event"] == "quit")
        .expect("trace must contain quit");
    assert_eq!(quit["detail"]["dirty_buffers"], 0);
    assert_eq!(quit["detail"]["will_quit"], true);
}

fn run_save_sequence(label: &str, input: &[u8], expected: &str, on_save_hook: bool) -> TestResult {
    let project = TempProject::new(label);
    if on_save_hook {
        project.write(
            "catomic/config.toml",
            "[commands.saved]\ncommand = \"printf saved\"\n[hooks]\non_save = [\"saved\"]\n",
        );
    }
    let active = project.write("note.txt", "");
    let trace_path = project.root.join("save-trace.jsonl");
    let mut editor = spawn_traced(&active, &project.root, &trace_path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(input)?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, expected);
    let trace = read_trace(&trace_path)?;
    assert_clean_save_then_quit(&trace);
    if on_save_hook {
        assert!(trace
            .iter()
            .any(|entry| { entry["event"] == "hook" && entry["detail"]["stage"] == "queued" }));
    }
    Ok(())
}

#[test]
fn pty_rapid_save_quit_sequences_keep_one_exact_save_point() -> TestResult {
    run_save_sequence("rapid_save", b"x\x13\x11", "x", false)?;
    run_save_sequence(
        "bracketed_paste_save",
        b"\x1b[200~pasted\x1b[201~\x13\x11",
        "pasted",
        false,
    )?;
    run_save_sequence("undo_redo_save", b"x\x1a\x19\x13\x11", "x", false)?;
    run_save_sequence("hooked_save", b"h\x13\x11", "h", true)
}

#[test]
fn pty_post_save_edit_is_traced_and_remains_dirty() -> TestResult {
    let project = TempProject::new("post_save_edit_trace");
    let active = project.write("note.txt", "");
    let trace_path = project.root.join("save-trace.jsonl");
    let mut editor = spawn_traced(&active, &project.root, &trace_path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"x\x13y\x11")?;
    editor.wait_for_output(
        "post-save mutation quit guard",
        "Unsaved changes. Press Ctrl+Q again",
    )?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "x");
    let trace = read_trace(&trace_path)?;
    let save_end = trace
        .iter()
        .position(|entry| entry["event"] == "save_end")
        .expect("trace must contain save_end");
    assert_eq!(trace[save_end]["state"]["dirty"], false);
    assert!(trace[save_end + 1..]
        .iter()
        .any(|entry| entry["event"] == "content_edit"));
    let quit = trace
        .iter()
        .find(|entry| entry["event"] == "quit")
        .expect("trace must contain first quit");
    assert_eq!(quit["detail"]["dirty_buffers"], 1);
    assert_eq!(quit["detail"]["will_quit"], false);
    assert_ne!(
        quit["state"]["history_position"],
        quit["state"]["saved_history_position"]
    );
    Ok(())
}

#[test]
fn pty_save_as_records_the_new_target_and_quits_clean() -> TestResult {
    let project = TempProject::new("save_as_trace");
    let source = project.write("source.txt", "");
    let target = project.root.join("saved-as.txt");
    let trace_path = project.root.join("save-trace.jsonl");
    let mut editor = spawn_traced(&source, &project.root, &trace_path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"x\x1b[80;6u")?; // Edit, then Ctrl+Shift+P via CSI-u.
    editor.wait_for_output("Save As command prompt", "Command: ")?;
    editor.send_keys(format!("save as {}\r\x11", target.display()).as_bytes())?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&source)?, "");
    assert_eq!(fs::read_to_string(&target)?, "x");
    let trace = read_trace(&trace_path)?;
    assert_clean_save_then_quit(&trace);
    let begin = trace
        .iter()
        .find(|entry| entry["event"] == "save_begin")
        .expect("trace must contain save_begin");
    assert_eq!(begin["target"].as_str(), target.to_str());
    Ok(())
}

#[cfg(unix)]
#[test]
fn pty_duplicate_path_alias_reuses_the_dirty_buffer_before_save() -> TestResult {
    use std::os::unix::fs::symlink;

    let project = TempProject::new("duplicate_alias_trace");
    let source = project.write("source.txt", "alpha");
    let alias = project.root.join("alias.txt");
    symlink(&source, &alias)?;
    let trace_path = project.root.join("save-trace.jsonl");
    let mut editor = spawn_traced(&source, &project.root, &trace_path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"X\x0f")?; // Edit the source, then Ctrl+O.
    editor.wait_for_output("alias open prompt", "Open file: ")?;
    editor.send_keys(format!("{}\r", alias.display()).as_bytes())?;
    editor.wait_for_output("alias buffer reuse", "Already open:")?;
    editor.wait_for_output("dirty source remains active", "Xalpha")?;

    // This exact pair used to save a second clean alias buffer, then warn about
    // the still-dirty original buffer on Ctrl+Q.
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(source)?, "Xalpha");
    let trace = read_trace(&trace_path)?;
    assert_clean_save_then_quit(&trace);
    let begin = trace
        .iter()
        .find(|entry| entry["event"] == "save_begin")
        .expect("trace must contain save_begin");
    assert_eq!(begin["state"]["buffer_count"], 1);
    assert_eq!(begin["state"]["buffer_index"], 0);
    Ok(())
}
