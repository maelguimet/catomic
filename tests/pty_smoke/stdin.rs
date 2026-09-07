use super::*;

fn piped_editor(bytes: &[u8], argument: &str) -> TestResult<PtyEditor> {
    let environment = TempProject::new("stdin_environment");
    let input = environment.root.join("input");
    fs::write(&input, bytes)?;
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "/bin/cat \"$1\" | \"$2\" \"$3\"",
        "catomic-stdin-fixture",
    ]);
    command.arg(&input);
    command.arg(env!("CARGO_BIN_EXE_catomic"));
    command.arg(argument);
    command.env("XDG_CONFIG_HOME", &environment.root);
    command.env("XDG_STATE_HOME", &environment.root);
    command.env("HOME", &environment.root);
    command.env("LC_ALL", "C.UTF-8");
    command.env("TERM", "xterm-256color");
    PtyEditor::spawn_command_with_environment(command, environment)
}

#[test]
fn piped_stdin_accepts_terminal_typing_undo_save_as_and_clean_quit() -> TestResult {
    let saved = TempPath::new("stdin_saved");
    let original = "\u{feff}piped 猫\r\n";
    let mut editor = piped_editor(original.as_bytes(), "-")?;
    editor.wait_for_initial_render()?;
    editor.wait_for_output("stdin content", "piped 猫")?;
    editor.send_keys(b"X\x1a\x11")?;
    editor.wait_for_output(
        "import remains unsaved after undo",
        "Unsaved changes. Press Ctrl+Q again to quit without saving",
    )?;
    editor.send_keys(b"\x13")?;
    editor.send_keys(saved.path.to_str().ok_or("non-UTF-8 test path")?.as_bytes())?;
    editor.send_keys(b"\r")?;
    wait_until("stdin Save As", Duration::from_secs(2), || {
        fs::read(&saved.path).is_ok_and(|bytes| bytes == original.as_bytes())
    })?;
    editor.send_keys(b"Y\x1a\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read(&saved.path)?, original.as_bytes());
    Ok(())
}

#[test]
fn piped_stdin_without_an_edit_still_requires_dirty_quit_confirmation() -> TestResult {
    let mut editor = piped_editor(b"unsaved import", "-")?;
    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_output(
        "initial stdin dirty quit",
        "Unsaved changes. Press Ctrl+Q again to quit without saving",
    )?;
    assert!(editor.child.try_wait()?.is_none());
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn piped_stdin_rejects_invalid_utf8_before_terminal_setup() -> TestResult {
    let mut editor = piped_editor(b"valid prefix\n\xff", "-")?;
    editor.wait_for_exit_code(1)?;
    editor.wait_for_output("invalid stdin", "standard input is not valid UTF-8")?;
    assert!(!editor.output_string().contains("\x1b[?1049h"));
    Ok(())
}

#[test]
fn redirected_stdin_read_error_precedes_terminal_setup() -> TestResult {
    let environment = TempProject::new("stdin_read_error");
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "exec \"$1\" - < \"$2\"", "catomic-stdin-fixture"]);
    command.arg(env!("CARGO_BIN_EXE_catomic"));
    command.arg(&environment.root);
    command.env("XDG_CONFIG_HOME", &environment.root);
    command.env("XDG_STATE_HOME", &environment.root);
    command.env("HOME", &environment.root);
    command.env("LC_ALL", "C.UTF-8");
    command.env("TERM", "xterm-256color");
    let mut editor = PtyEditor::spawn_command_with_environment(command, environment)?;
    editor.wait_for_exit_code(1)?;
    editor.wait_for_output("stdin read error", "read standard input:")?;
    assert!(!editor.output_string().contains("\x1b[?1049h"));
    Ok(())
}

#[test]
fn stdin_from_a_terminal_is_rejected_without_entering_raw_mode() -> TestResult {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.arg("-");
    let mut editor = PtyEditor::spawn_command(command)?;
    editor.wait_for_exit_code(1)?;
    editor.wait_for_output(
        "stdin needs redirection",
        "requires piped or redirected standard input",
    )?;
    assert!(!editor.output_string().contains("\x1b[?1049h"));
    Ok(())
}

#[test]
fn ordinary_file_startup_leaves_piped_input_for_the_next_reader() -> TestResult {
    let environment = TempProject::new("stdin_not_requested");
    let file = environment.write("named.txt", "named file content");
    let remaining = environment.root.join("remaining");
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "printf '%s' 'untouched pipe' | { \"$1\" \"$2\"; /bin/cat > \"$3\"; }",
        "catomic-stdin-fixture",
    ]);
    command.arg(env!("CARGO_BIN_EXE_catomic"));
    command.arg(&file);
    command.arg(&remaining);
    command.env("XDG_CONFIG_HOME", &environment.root);
    command.env("XDG_STATE_HOME", &environment.root);
    command.env("HOME", &environment.root);
    command.env("LC_ALL", "C.UTF-8");
    command.env("TERM", "xterm-256color");
    let mut editor = PtyEditor::spawn_command_with_environment(command, environment)?;
    editor.wait_for_initial_render()?;
    editor.wait_for_output("named content", "named file content")?;
    assert!(!editor.output_string().contains("untouched pipe"));
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read(&remaining)?, b"untouched pipe");
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn stdin_waiting_for_a_producer_can_be_cancelled_from_the_controlling_terminal() -> TestResult {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let environment = TempProject::new("stdin_cancel");
    let fifo = environment.root.join("input.fifo");
    let path = CString::new(fifo.as_os_str().as_encoded_bytes())?;
    // SAFETY: path is a terminated fixture pathname; mkfifo creates only that FIFO.
    if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "exec \"$1\" - < \"$2\"", "catomic-stdin-fixture"]);
    command.arg(env!("CARGO_BIN_EXE_catomic"));
    command.arg(&fifo);
    command.env("XDG_CONFIG_HOME", &environment.root);
    command.env("XDG_STATE_HOME", &environment.root);
    command.env("HOME", &environment.root);
    command.env("LC_ALL", "C.UTF-8");
    command.env("TERM", "xterm-256color");
    let mut editor = PtyEditor::spawn_command_with_environment(command, environment)?;
    let mut producer = None;
    wait_until("stdin FIFO reader", Duration::from_secs(2), || {
        producer = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
            .ok();
        producer.is_some()
    })?;
    let mut producer = producer.ok_or("FIFO producer did not open")?;
    producer.write_all(b"partial import")?;
    wait_until(
        "stdin consumes partial input",
        Duration::from_secs(2),
        || {
            let mut unread: libc::c_int = 0;
            // SAFETY: this live fixture FIFO accepts FIONREAD and unread is writable.
            unsafe {
                libc::ioctl(producer.as_raw_fd(), libc::FIONREAD, &mut unread) == 0 && unread == 0
            }
        },
    )?;
    editor.send_keys(b"\x03")?;
    editor.wait_for_exit_code(130)?;
    assert!(!editor.output_string().contains("\x1b[?1049h"));
    // Keep the producer open through exit: cancellation cannot depend on EOF.
    drop(producer);
    Ok(())
}
