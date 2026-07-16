//! Purpose: run one explicitly requested Project linter without blocking editor input.
//! Owns: safe `{file}` shell substitution, child lifetime, bounded output capture, and polling.
//! Must not: load config, mutate App/buffers/files, run automatically, index projects, or network.
//! Invariants: output memory is capped; dropping a live task requests child termination.
//! Phase: 5-c on-demand linter process runner.

use std::io;
use std::path::Path;
use std::time::Duration;

use crate::external::{ExternalCommandResult, ExternalCommandTask};

#[cfg(test)]
use crate::external::substitute_file;

const LINTER_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) enum LinterResult {
    Finished { output: String, code: Option<i32> },
    Cancelled,
    Error(String),
}

pub(crate) struct LinterTask {
    task: ExternalCommandTask,
}

impl LinterTask {
    pub(crate) fn start(command: &str, cwd: &Path) -> io::Result<Self> {
        ExternalCommandTask::start(command, cwd, Vec::new(), LINTER_TIMEOUT)
            .map(|task| Self { task })
    }

    pub(crate) fn try_result(&mut self) -> Option<LinterResult> {
        self.task.try_result().map(map_result)
    }
}

fn map_result(result: ExternalCommandResult) -> LinterResult {
    let ExternalCommandResult::Finished {
        stdout,
        stderr,
        code,
        truncated,
    } = result
    else {
        return match result {
            ExternalCommandResult::TimedOut => {
                LinterResult::Error("linter timed out after 120 seconds".to_string())
            }
            ExternalCommandResult::Cancelled => LinterResult::Cancelled,
            ExternalCommandResult::Error(error) => LinterResult::Error(error),
            ExternalCommandResult::Finished { .. } => unreachable!(),
        };
    };
    let mut output = stdout;
    if !output.is_empty() && !output.ends_with('\n') && !stderr.is_empty() {
        output.push('\n');
    }
    output.push_str(&stderr);
    if truncated {
        output.push_str("\n[catomic: linter output truncated]\n");
    }
    LinterResult::Finished { output, code }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn file_placeholder_is_shell_quoted() {
        assert_eq!(
            substitute_file("tool {file}", Path::new("/tmp/a b's.rs")),
            "tool '/tmp/a b'\"'\"'s.rs'"
        );
    }

    #[test]
    fn task_captures_stdout_stderr_and_exit_status() {
        let mut task = LinterTask::start(
            "printf 'a.rs:2:3: warning: hi\\n'; printf 'b.rs:1:1: error: bad\\n' >&2; exit 7",
            Path::new("/tmp"),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = task.try_result() {
                break result;
            }
            assert!(Instant::now() < deadline, "linter task timed out");
            std::thread::sleep(Duration::from_millis(5));
        };

        let LinterResult::Finished { output, code } = result else {
            panic!("unexpected linter result");
        };
        assert_eq!(code, Some(7));
        assert!(output.contains("a.rs:2:3"));
        assert!(output.contains("b.rs:1:1"));
    }
}
