//! Runs setup-wizard install commands (`brew install ollama`, `tailscale
//! up`, ...) in the background and buffers their output so the browser can
//! poll progress instead of blocking on a long-running request. Mirrors the
//! `LiveOutputRegistry` pattern used for `session_bash` output in
//! `crate::live_output`.

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

    /// Spawns `command` under `sh -c`, streaming its combined stdout/stderr
    /// into the registry as it runs. Returns `false` without spawning if this
    /// job is already in flight. Stdin is closed so a command that needs
    /// interactive input (e.g. a sudo password) fails fast and prints why,
    /// instead of hanging forever — the command text is always shown in the
    /// wizard too, so the user can copy it into their own terminal instead.
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

    fn append(&self, job_id: &str, line: &str) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(job_id) {
            job.output.push_str(line);
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
}
