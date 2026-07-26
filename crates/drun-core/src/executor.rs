//! Runs one shell command inside a `Sandbox`-confined workspace: applies the
//! overlay symlinks, enforces the timeout, and tracks every in-flight child's
//! process group so the daemon can kill them all on shutdown.

use crate::sandbox::Sandbox;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

static RUNNING_CHILD_PGIDS: OnceLock<Mutex<std::collections::HashSet<i32>>> = OnceLock::new();

struct ChildGuard(i32);

impl ChildGuard {
    fn new(pgid: i32) -> Self {
        BashExecutor::running_child_pgids()
            .lock()
            .unwrap()
            .insert(pgid);
        Self(pgid)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        BashExecutor::running_child_pgids()
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

pub(crate) struct BashOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_code: Option<i32>,
}

pub struct BashExecutor;

impl BashExecutor {
    fn running_child_pgids() -> &'static Mutex<std::collections::HashSet<i32>> {
        RUNNING_CHILD_PGIDS.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
    }

    /// Symlinks `overlays` into `workspace_dir`, spawns `command` sandboxed
    /// against it (confined to `workspace_dir` plus `read_paths`), and waits
    /// for it to finish or hit `timeout_ms`.
    pub(crate) fn run(
        workspace_dir: &Path,
        overlays: &HashMap<String, PathBuf>,
        read_paths: Vec<PathBuf>,
        command: &str,
        timeout_ms: u64,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<BashOutput> {
        for (key, host_path) in overlays {
            let dest = workspace_dir.join(key);
            if !dest.exists() {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(host_path, &dest)?;
            }
        }

        let scratch_dir = tempfile::TempDir::new()?;
        let child = Sandbox::new(workspace_dir, scratch_dir.path(), read_paths)
            .command(command)?
            .current_dir(workspace_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        Self::run_child(child, timeout_ms, on_stdout)
    }

    fn run_child(
        mut child: Child,
        timeout_ms: u64,
        on_stdout: &mut dyn FnMut(String),
    ) -> anyhow::Result<BashOutput> {
        let pgid = child.id() as i32;
        let _pgid_guard = ChildGuard::new(pgid);
        let child_stderr = child.stderr.take().unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let child = Arc::new(Mutex::new(child));
        let child_for_timeout = Arc::clone(&child);
        let timed_out = Arc::new(AtomicBool::new(false));
        let timed_out_flag = Arc::clone(&timed_out);
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if cancel_rx
                .recv_timeout(Duration::from_millis(timeout_ms))
                .is_err()
            {
                timed_out_flag.store(true, Ordering::Relaxed);
                Self::kill_process_tree(pgid);
                let _ = child_for_timeout.lock().unwrap().kill();
            }
        });
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = BufReader::new(child_stderr).read_to_string(&mut buf);
            buf
        });
        let mut stdout = String::new();
        let mut stdout_reader = BufReader::new(child_stdout);
        loop {
            let mut line = String::new();
            match stdout_reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    on_stdout(line.trim_end_matches('\n').to_string());
                    stdout.push_str(&line);
                }
            }
        }
        let _ = cancel_tx.send(());
        let stderr = stderr_thread.join().unwrap_or_default();
        let exit_code = child
            .lock()
            .unwrap()
            .wait()
            .ok()
            .and_then(|status| status.code());
        Self::kill_process_tree(pgid);
        if timed_out.load(Ordering::Relaxed) {
            return Err(crate::error::RunnerError::timeout(timeout_ms).into());
        }
        Ok(BashOutput {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// Kills every sandboxed child (and its descendants) currently tracked as
    /// running. Intended for the daemon's shutdown handler, so an in-flight
    /// `session_bash` call doesn't outlive the daemon process.
    pub fn kill_all_running_children() {
        let pgids: Vec<i32> = Self::running_child_pgids()
            .lock()
            .unwrap()
            .iter()
            .copied()
            .collect();
        for pgid in pgids {
            Self::kill_process_tree(pgid);
        }
    }

    #[cfg(unix)]
    fn kill_process_tree(pid: i32) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        for descendant_pid in Self::descendant_pids(pid) {
            unsafe {
                libc::kill(descendant_pid, libc::SIGKILL);
            }
        }
    }

    #[cfg(not(unix))]
    fn kill_process_tree(_pid: i32) {}

    #[cfg(unix)]
    fn descendant_pids(root_pid: i32) -> Vec<i32> {
        let Ok(output) = std::process::Command::new("ps")
            .args(["-Ao", "pid=,ppid="])
            .output()
        else {
            return Vec::new();
        };
        let parent_of: Vec<(i32, i32)> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid: i32 = fields.next()?.parse().ok()?;
                let ppid: i32 = fields.next()?.parse().ok()?;
                Some((pid, ppid))
            })
            .collect();

        let mut descendants = std::collections::HashSet::new();
        let mut frontier = vec![root_pid];
        while let Some(parent_pid) = frontier.pop() {
            for &(pid, ppid) in &parent_of {
                if ppid == parent_pid && descendants.insert(pid) {
                    frontier.push(pid);
                }
            }
        }
        descendants.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn descendant_pids_finds_a_grandchild_process() {
        // A plain "sleep 5" tail-call-execs into sh's own pid, so force a
        // genuine subshell fork to get a real two-level descendant chain.
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("(sleep 5 & wait) & wait")
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));

        let descendants = BashExecutor::descendant_pids(child.id() as i32);
        assert_eq!(descendants.len(), 2, "expected a subshell and its sleep");

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn kill_all_running_children_kills_a_registered_process_group() {
        use std::os::unix::process::CommandExt;
        // Sandboxed children are spawned as their own process-group leader
        // (see sandbox.rs) so kill_process_tree's `-pgid` reaches them.
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .process_group(0)
            .spawn()
            .unwrap();
        let pgid = child.id() as i32;
        let guard = ChildGuard::new(pgid);

        BashExecutor::kill_all_running_children();

        let status = child.wait().unwrap();
        assert!(
            !status.success(),
            "child should have been killed, not exited cleanly"
        );

        drop(guard);
        assert!(
            !BashExecutor::running_child_pgids()
                .lock()
                .unwrap()
                .contains(&pgid),
            "dropping the guard should unregister the pgid"
        );
    }
}
