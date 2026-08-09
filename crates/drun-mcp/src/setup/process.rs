//! Runs setup-wizard shell commands in the background and buffers their
//! output for polling, mirroring `crate::live_output::LiveOutputRegistry`.

use serde::Serialize;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct JobState {
    pub(crate) command: String,
    pub(crate) output: String,
    pub(crate) running: bool,
    pub(crate) exit_code: Option<i32>,
}

#[derive(Clone, Default)]
pub(crate) struct JobRegistry {
    jobs: Arc<Mutex<HashMap<String, JobState>>>,
}

impl JobRegistry {
    pub(crate) fn snapshot(&self, job_id: &str) -> Option<JobState> {
        self.jobs.lock().unwrap().get(job_id).cloned()
    }

    fn is_running(&self, job_id: &str) -> bool {
        self.jobs
            .lock()
            .unwrap()
            .get(job_id)
            .is_some_and(|job| job.running)
    }

    /// Spawns `command` under `sh -c`, streaming stdout/stderr into the
    /// registry. Returns `false` without spawning if the job is already
    /// running. Stdin is closed, so a command needing a password fails fast
    /// instead of hanging — its text is always shown so it can be copied
    /// into a real terminal instead.
    pub(crate) fn start(&self, job_id: &str, command: &str) -> bool {
        if self.is_running(job_id) {
            return false;
        }
        self.jobs.lock().unwrap().insert(
            job_id.to_string(),
            JobState {
                command: command.to_string(),
                output: String::new(),
                running: true,
                exit_code: None,
            },
        );

        let registry = self.clone();
        let job_id = job_id.to_string();
        let command = command.to_string();
        tokio::spawn(async move { registry.run(&job_id, &command).await });
        true
    }

    async fn run(&self, job_id: &str, command: &str) {
        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                self.append(job_id, &format!("failed to start: {error}"));
                self.finish(job_id, None);
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");
        let stdout_task = tokio::spawn(Self::pump(stdout, self.clone(), job_id.to_string()));
        let stderr_task = tokio::spawn(Self::pump(stderr, self.clone(), job_id.to_string()));
        let _ = tokio::join!(stdout_task, stderr_task);

        let exit_code = child.wait().await.ok().and_then(|status| status.code());
        self.finish(job_id, exit_code);
    }

    async fn pump(pipe: impl AsyncRead + Unpin, registry: JobRegistry, job_id: String) {
        let mut lines = BufReader::new(pipe).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            registry.append(&job_id, &line);
        }
    }

    /// Strips ANSI control sequences before storing — progress bars redraw
    /// with cursor-move/sync codes that print as literal garbage outside a
    /// real terminal. Lines that are pure escape-sequence noise vanish
    /// entirely once stripped and are dropped rather than kept as blanks.
    fn append(&self, job_id: &str, line: &str) {
        let cleaned = strip_ansi_sequences(line);
        if cleaned.trim().is_empty() {
            return;
        }
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.output.push_str(&cleaned);
            job.output.push('\n');
        }
    }

    fn finish(&self, job_id: &str, exit_code: Option<i32>) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.running = false;
            job.exit_code = exit_code;
        }
    }
}

/// Drops ANSI CSI (`ESC [ ... <letter>`) and OSC (`ESC ] ... BEL`)
/// sequences. Hand-rolled rather than pulling in a regex crate for one
/// small, fixed grammar.
fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.clone().next() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\u{7}' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn wait_until_finished(registry: &JobRegistry, job_id: &str) -> JobState {
        for _ in 0..100 {
            if let Some(state) = registry.snapshot(job_id)
                && !state.running
            {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("job '{job_id}' did not finish in time");
    }

    #[tokio::test]
    async fn snapshot_is_none_for_a_job_that_never_ran() {
        let registry = JobRegistry::default();
        assert_eq!(registry.snapshot("missing"), None);
    }

    #[tokio::test]
    async fn start_reports_running_immediately_with_the_given_command() {
        let registry = JobRegistry::default();
        assert!(registry.start("job1", "echo hi"));

        let state = registry.snapshot("job1").unwrap();
        assert_eq!(state.command, "echo hi");
        assert!(state.running);
    }

    #[tokio::test]
    async fn a_second_start_while_running_is_rejected() {
        let registry = JobRegistry::default();
        assert!(registry.start("job1", "sleep 0.2"));
        assert!(!registry.start("job1", "echo should not run"));
    }

    #[tokio::test]
    async fn finished_job_captures_output_and_a_zero_exit_code() {
        let registry = JobRegistry::default();
        registry.start("job1", "echo line one; echo line two");

        let state = wait_until_finished(&registry, "job1").await;
        assert_eq!(state.exit_code, Some(0));
        assert_eq!(state.output, "line one\nline two\n");
    }

    #[tokio::test]
    async fn a_failing_command_captures_a_nonzero_exit_code() {
        let registry = JobRegistry::default();
        registry.start("job1", "exit 3");

        let state = wait_until_finished(&registry, "job1").await;
        assert_eq!(state.exit_code, Some(3));
    }

    #[tokio::test]
    async fn a_finished_job_can_be_started_again() {
        let registry = JobRegistry::default();
        registry.start("job1", "echo first");
        wait_until_finished(&registry, "job1").await;

        assert!(registry.start("job1", "echo second"));
        let state = wait_until_finished(&registry, "job1").await;
        assert_eq!(state.output, "second\n");
    }

    #[test]
    fn strip_ansi_sequences_removes_a_synchronized_update_progress_redraw() {
        // A representative single line of `ollama pull`'s progress output:
        // synchronized-update wrapper, cursor-to-column-1, the human text,
        // "erase to end of line", then cursor shown/sync-off again.
        let raw =
            "\u{1b}[?2026h\u{1b}[?25l\u{1b}[1Gpulling manifest \u{1b}[K\u{1b}[?25h\u{1b}[?2026l";
        assert_eq!(strip_ansi_sequences(raw), "pulling manifest ");
    }

    #[test]
    fn strip_ansi_sequences_removes_an_osc_sequence() {
        let raw = "\u{1b}]0;window title\u{7}visible text";
        assert_eq!(strip_ansi_sequences(raw), "visible text");
    }

    #[test]
    fn strip_ansi_sequences_leaves_plain_text_untouched() {
        assert_eq!(
            strip_ansi_sequences("pulling f5ee307a2982: 12%"),
            "pulling f5ee307a2982: 12%"
        );
    }

    #[tokio::test]
    async fn a_line_that_is_pure_ansi_noise_is_dropped_instead_of_stored_blank() {
        let registry = JobRegistry::default();
        // The spinner-only redraw a bare `\u{1b}[?2026h...\u{1b}[?2026l` frame
        // becomes empty once stripped, and should never show up as a blank
        // line in the wizard's output panel.
        registry.start(
            "job1",
            "printf 'kept line\\n\\033[?2026h\\033[?25l\\033[1G\\033[K\\033[?25h\\033[?2026l\\n'",
        );

        let state = wait_until_finished(&registry, "job1").await;
        assert_eq!(state.output, "kept line\n");
    }
}
