use super::*;

#[cfg(unix)]
#[test]
fn pty_saved_config_detour_closes_and_preserves_dirty_source() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new("config_command");
    let active = project.write("note.txt", "source stays untouched");
    let config = project.root.join("catomic/config.toml");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"X")?;
    editor.wait_for_output("dirty source buffer", "Xsource stays untouched")?;
    editor.send_keys(&[keys::CTRL_SHIFT_P, b"config", keys::ENTER].concat())?;
    editor.wait_for_output("config creation confirmation", "Type yes to confirm")?;
    assert!(!config.exists(), "prompt must not create configuration");

    editor.send_keys(&[b"yes", keys::ENTER].concat())?;
    editor.wait_for_output("config template buffer", "Catomic configuration")?;
    let created = fs::read_to_string(&config)?;
    assert!(created.contains("[theme.colors]"));
    assert!(created.contains("[keybindings]"));
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);

    editor.send_keys(&[b"#", keys::CTRL_S].concat())?;
    editor.wait_for_output(
        "configuration save policy",
        "Saved configuration. Restart Catomic to apply configuration changes.",
    )?;
    assert!(fs::read_to_string(&config)?.starts_with("## Catomic configuration"));

    editor.clear_output();
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_output("restored dirty source", "Xsource stays untouched")?;
    let close_output = editor.output_string();
    assert!(!close_output.contains("configuration remains open"));
    assert!(!close_output.contains("Buffer closed."));
    assert!(!close_output.contains("file 1/2"));

    let cycle_start = editor.output_len();
    editor.send_keys(keys::ALT_PAGE_DOWN)?;
    wait_until("sole-buffer cycle frame", Duration::from_secs(2), || {
        editor.output_since(cycle_start).contains("\x1b[?2026l")
    })?;
    let cycle_frame = editor.output_since(cycle_start);
    assert!(!cycle_frame.contains("Catomic configuration"));

    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_output(
        "dirty source quit guard",
        "Unsaved changes. Press Ctrl+Q again to quit without saving",
    )?;
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_exit()?;
    let output = editor.output_string();
    assert!(output.contains("\x1b[0m"), "terminal styles must reset");
    assert!(
        output.contains("\x1b]112\x07"),
        "terminal cursor color must reset"
    );
    assert_eq!(fs::read_to_string(active)?, "source stays untouched");
    Ok(())
}

#[test]
fn pty_dirty_config_detour_refuses_then_discards_only_config_and_reopens_from_disk() -> TestResult {
    let project = TempProject::new("dirty_config_detour");
    let config = project.write("catomic/config.toml", "# CONFIG DISK MARKER\n");
    let active = project.write("note.txt", "SOURCE BUFFER MARKER");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(&[keys::CTRL_SHIFT_P, b"config", keys::ENTER].concat())?;
    editor.wait_for_output("existing config detour", "CONFIG DISK MARKER")?;
    editor.send_keys(b"X")?;
    editor.wait_for_output("dirty config edit", "X\x1b[96m# CONFIG DISK MARKER")?;

    editor.clear_output();
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_output(
        "config-local discard guard",
        "Unsaved configuration. Press Ctrl+Q again to discard it, or Ctrl+S to save.",
    )?;
    let guard_frame = editor.output_string();
    assert!(guard_frame.contains("Unsaved configuration."));
    assert!(!guard_frame.contains("SOURCE BUFFER MARKER"));

    editor.clear_output();
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_output(
        "source restored after config discard",
        "SOURCE BUFFER MARKER",
    )?;
    assert_eq!(fs::read_to_string(&config)?, "# CONFIG DISK MARKER\n");
    assert!(!editor.output_string().contains("CONFIG DISK MARKER"));
    assert!(!editor.output_string().contains("file 1/2"));

    editor.clear_output();
    editor.send_keys(&[keys::CTRL_SHIFT_P, b"config", keys::ENTER].concat())?;
    editor.wait_for_output("fresh config reopened from disk", "# CONFIG DISK MARKER")?;
    assert!(!editor.output_string().contains("X# CONFIG DISK MARKER"));

    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_output(
        "source restored after fresh config close",
        "SOURCE BUFFER MARKER",
    )?;
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_clean_config_detour_returns_to_invoker_in_multi_buffer_ring() -> TestResult {
    let project = TempProject::new("multi_buffer_config_detour");
    project.write("catomic/config.toml", "# CONFIG MUST LEAVE RING\n");
    let first = project.write("first.txt", "FIRST BUFFER MARKER");
    let second = project.write("second.txt", "SECOND BUFFER MARKER");
    let mut editor = PtyEditor::spawn_with_xdg(&first, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(
        &[
            keys::CTRL_SHIFT_P,
            format!("open {}", second.display()).as_bytes(),
            keys::ENTER,
        ]
        .concat(),
    )?;
    editor.wait_for_output("second source buffer", "SECOND BUFFER MARKER")?;
    editor.send_keys(&[keys::CTRL_SHIFT_P, b"config", keys::ENTER].concat())?;
    editor.wait_for_output("clean config detour", "CONFIG MUST LEAVE RING")?;

    editor.clear_output();
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_output("return to invoking second buffer", "SECOND BUFFER MARKER")?;
    editor.wait_for_output("config removed from three-buffer ring", "file 2/2")?;
    assert!(!editor.output_string().contains("CONFIG MUST LEAVE RING"));

    editor.clear_output();
    editor.send_keys(keys::ALT_PAGE_DOWN)?;
    editor.wait_for_output("cycle to first source", "FIRST BUFFER MARKER")?;
    editor.wait_for_output("first of two remaining buffers", "file 1/2")?;
    assert!(!editor.output_string().contains("CONFIG MUST LEAVE RING"));

    editor.clear_output();
    editor.send_keys(keys::ALT_PAGE_DOWN)?;
    editor.wait_for_output("cycle back to second source", "SECOND BUFFER MARKER")?;
    editor.wait_for_output("second of two remaining buffers", "file 2/2")?;
    assert!(!editor.output_string().contains("CONFIG MUST LEAVE RING"));

    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_bare_config_and_edit_alias_open_the_resolved_file_in_catomic() -> TestResult {
    let project = TempProject::new("config_cli_existing");
    let config = project.write(
        "catomic/config.toml",
        "# exact bare config content\n[theme]\nname = \"invalid-but-editable\"\n",
    );
    let before = fs::read(&config)?;

    for arguments in [["config", ""], ["config", "edit"]] {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        command.cwd(&project.root);
        command.arg(arguments[0]);
        if !arguments[1].is_empty() {
            command.arg(arguments[1]);
        }
        let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

        editor.wait_for_initial_render()?;
        editor.wait_for_output("resolved config content", "exact bare config content")?;
        editor.send_keys(keys::CTRL_Q)?;
        editor.wait_for_exit()?;
    }

    assert_eq!(fs::read(&config)?, before);
    assert!(!project.root.join("config").exists());
    assert!(!project.root.join("update").exists());
    Ok(())
}

#[test]
fn pty_dirty_config_opened_from_shell_keeps_normal_guarded_session_quit() -> TestResult {
    let project = TempProject::new("config_cli_dirty_quit");
    let config = project.write("catomic/config.toml", "# shell config remains on disk\n");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("config");
    let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

    editor.wait_for_output("shell config content", "shell config remains on disk")?;
    editor.send_keys(&[b"X", keys::CTRL_Q].concat())?;
    editor.wait_for_output(
        "ordinary global quit guard",
        "Unsaved changes. Press Ctrl+Q again to quit without saving",
    )?;
    assert!(!editor.output_string().contains("Unsaved configuration."));
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_exit()?;
    assert_eq!(
        fs::read_to_string(config)?,
        "# shell config remains on disk\n"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn pty_bare_config_confirms_and_opens_a_missing_private_template() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new("config_cli_missing");
    let config = project.root.join("catomic/config.toml");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("config");
    let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

    editor.wait_for_output("config CLI creation confirmation", "Type yes to confirm")?;
    assert!(!config.exists(), "prompt must not create configuration");
    assert!(!project.root.join("config").exists());
    editor.send_keys(&[b"yes", keys::ENTER].concat())?;
    editor.wait_for_output("config CLI template", "Catomic configuration")?;
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(config.parent().expect("config parent"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    editor.send_keys(keys::CTRL_Q)?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_config_edit_decline_exits_cleanly_without_creating_a_file() -> TestResult {
    let project = TempProject::new("config_cli_decline");
    let config = project.root.join("catomic/config.toml");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("config");
    command.arg("edit");
    let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

    editor.wait_for_output("config edit creation confirmation", "Type yes to confirm")?;
    editor.send_keys(&[b"no", keys::ENTER].concat())?;
    editor.wait_for_exit()?;
    assert!(!config.exists());
    assert!(!project.root.join("config").exists());
    Ok(())
}
