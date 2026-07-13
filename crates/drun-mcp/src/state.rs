//! Serializable view types for session and checkpoint state. Each type owns
//! the logic that builds it from live session data.

use crate::handler::DrunHandler;
use drun_core::{CheckpointRef, Config, FileMap, Session, SnapshotMeta};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SessionSummary {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_id: usize,
    checkpoint_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
}

impl SessionSummary {
    pub(crate) fn all(sessions: &HashMap<String, Arc<Mutex<Session>>>) -> Vec<SessionSummary> {
        sessions
            .iter()
            .map(|(id, arc)| {
                let session = DrunHandler::lock_recovering(id, arc);
                let (parent_session_id, parent_checkpoint_id) =
                    CheckpointRef::split(&session.parent);
                SessionSummary {
                    session_id: id.clone(),
                    label: session.label.clone(),
                    checkpoint_id: session.current().id,
                    checkpoint_count: session.history().len(),
                    parent_session_id,
                    parent_checkpoint_id,
                }
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct CheckpointTreeNode {
    checkpoint_id: usize,
    is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    file_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    forks: Vec<SessionTreeNode>,
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SnapshotEntry {
    path: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_count: usize,
}

impl SnapshotEntry {
    pub(crate) fn catalog(snapshots_dir: &Path) -> Vec<SnapshotEntry> {
        let Ok(entries) = std::fs::read_dir(snapshots_dir) else {
            return vec![];
        };
        let mut catalog: Vec<SnapshotEntry> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let p = e.path();
                p.extension().and_then(|x| x.to_str()) == Some("meta")
                    && Path::new(p.file_stem().unwrap_or_default())
                        .extension()
                        .and_then(|x| x.to_str())
                        == Some("drun")
            })
            .filter_map(|e| {
                let meta_path = e.path();
                let drun_path = meta_path.with_extension("");
                let size_bytes = std::fs::metadata(&drun_path).ok()?.len();
                let meta = SnapshotMeta::decode(&std::fs::read(&meta_path).ok()?).ok()?;
                Some(SnapshotEntry {
                    path: drun_path.to_string_lossy().into_owned(),
                    size_bytes,
                    label: meta.label,
                    checkpoint_count: meta.checkpoint_count,
                })
            })
            .collect();
        catalog.sort_by(|a, b| a.path.cmp(&b.path));
        catalog
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DaemonStatus {
    version: &'static str,
    pid: u32,
    uptime_secs: u64,
    mcp_port: u16,
    web_port: u16,
    session_count: usize,
    max_sessions: Option<usize>,
    session_idle_timeout_secs: Option<u64>,
    max_workspace_mb: Option<u64>,
    max_checkpoints: Option<usize>,
    domain_allowlist: Vec<String>,
    mount_allowlist: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_rss_bytes: Option<u64>,
}

impl DaemonStatus {
    pub(crate) fn current(
        sessions: &HashMap<String, Arc<Mutex<Session>>>,
        config: &Config,
        started_at: Instant,
        mcp_port: u16,
        web_port: u16,
    ) -> DaemonStatus {
        DaemonStatus {
            version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
            uptime_secs: started_at.elapsed().as_secs(),
            mcp_port,
            web_port,
            session_count: sessions.len(),
            max_sessions: config.max_sessions,
            session_idle_timeout_secs: config.session_idle_timeout_secs,
            max_workspace_mb: config.max_workspace_mb,
            max_checkpoints: config.max_checkpoints,
            domain_allowlist: config.domain_allowlist.clone(),
            mount_allowlist: config
                .mount_allowlist
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            memory_rss_bytes: Self::memory_rss_bytes(),
        }
    }

    fn memory_rss_bytes() -> Option<u64> {
        let usage: libc::rusage = unsafe {
            let mut usage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
                return None;
            }
            usage
        };
        // ru_maxrss is bytes on macOS, kilobytes on Linux.
        let maxrss = usage.ru_maxrss as u64;
        Some(if cfg!(target_os = "macos") {
            maxrss
        } else {
            maxrss * 1024
        })
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SessionTreeNode {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    parent_session_id: Option<String>,
    parent_checkpoint_id: Option<usize>,
    age_secs: u64,
    idle_secs: u64,
    checkpoints: Vec<CheckpointTreeNode>,
}

impl SessionTreeNode {
    fn from_session(
        session_id: &str,
        session: &Session,
        children: &HashMap<(String, usize), Vec<(String, &Session)>>,
    ) -> SessionTreeNode {
        let current_id = session.current().id;
        let checkpoints = session
            .history()
            .iter()
            .map(|cp| {
                let forks = children
                    .get(&(session_id.to_string(), cp.id))
                    .map(|kids| {
                        kids.iter()
                            .map(|(id, s)| SessionTreeNode::from_session(id, s, children))
                            .collect()
                    })
                    .unwrap_or_default();
                CheckpointTreeNode {
                    checkpoint_id: cp.id,
                    is_current: cp.id == current_id,
                    label: cp.label.clone(),
                    stdout_bytes: cp.stdout.len(),
                    stderr_bytes: cp.stderr.len(),
                    file_count: cp.files.len(),
                    forks,
                }
            })
            .collect();
        let (parent_session_id, parent_checkpoint_id) = CheckpointRef::split(&session.parent);
        SessionTreeNode {
            session_id: session_id.to_string(),
            label: session.label.clone(),
            parent_session_id,
            parent_checkpoint_id,
            age_secs: session.created_at.elapsed().as_secs(),
            idle_secs: session.last_activity.elapsed().as_secs(),
            checkpoints,
        }
    }

    pub(crate) fn forest(sessions: &HashMap<String, Arc<Mutex<Session>>>) -> Vec<SessionTreeNode> {
        let locked_sessions: Vec<(String, std::sync::MutexGuard<Session>)> = sessions
            .iter()
            .map(|(id, arc)| (id.clone(), DrunHandler::lock_recovering(id, arc)))
            .collect();
        let mut children: HashMap<(String, usize), Vec<(String, &Session)>> = HashMap::new();
        let mut roots: Vec<(&str, &Session)> = Vec::new();

        for (id, session) in &locked_sessions {
            let session: &Session = session;
            let parent_exists = session
                .parent
                .as_ref()
                .is_some_and(|r| sessions.contains_key(&r.session_id));
            if parent_exists {
                let r = session.parent.as_ref().unwrap();
                children
                    .entry((r.session_id.clone(), r.checkpoint_id))
                    .or_default()
                    .push((id.clone(), session));
            } else {
                roots.push((id.as_str(), session));
            }
        }

        roots
            .into_iter()
            .map(|(id, session)| SessionTreeNode::from_session(id, session, &children))
            .collect()
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct SessionState {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_checkpoint_id: Option<usize>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    workspace_file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    committed_files: Vec<String>,
}

impl SessionState {
    pub(crate) fn compute(
        session_id: &str,
        session: &Session,
        previous_files: Option<&FileMap>,
        committed_files: Vec<String>,
    ) -> SessionState {
        let current = session.current();
        let delta = FileDelta::compute(previous_files, &current.files);
        let (parent_session_id, parent_checkpoint_id) = CheckpointRef::split(&session.parent);
        SessionState {
            session_id: session_id.to_string(),
            label: session.label.clone(),
            checkpoint_id: current.id,
            parent_session_id,
            parent_checkpoint_id,
            stdout_bytes: current.stdout.len(),
            stderr_bytes: current.stderr.len(),
            workspace_file_count: current.files.len(),
            files_added_count: delta.added.len(),
            files_modified_count: delta.modified.len(),
            files_removed_count: delta.removed.len(),
            committed_files,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct CheckpointSummary {
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
}

impl CheckpointSummary {
    pub(crate) fn history(session: &Session) -> Vec<CheckpointSummary> {
        let history = session.history();
        history
            .iter()
            .enumerate()
            .map(|(index, checkpoint)| {
                let previous_files = if index > 0 {
                    Some(&history[index - 1].files)
                } else {
                    None
                };
                let delta = FileDelta::compute(previous_files, &checkpoint.files);
                CheckpointSummary {
                    checkpoint_id: checkpoint.id,
                    label: checkpoint.label.clone(),
                    stdout_bytes: checkpoint.stdout.len(),
                    stderr_bytes: checkpoint.stderr.len(),
                    file_count: checkpoint.files.len(),
                    files_added_count: delta.added.len(),
                    files_modified_count: delta.modified.len(),
                    files_removed_count: delta.removed.len(),
                }
            })
            .collect()
    }
}

#[derive(Debug, PartialEq)]
struct FileDelta {
    added: Vec<String>,
    modified: Vec<String>,
    removed: Vec<String>,
}

impl FileDelta {
    fn compute(previous_files: Option<&FileMap>, current_files: &FileMap) -> FileDelta {
        let Some(previous) = previous_files else {
            return FileDelta {
                added: vec![],
                modified: vec![],
                removed: vec![],
            };
        };
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut removed = Vec::new();
        for key in current_files.keys() {
            if !previous.contains_key(key) {
                added.push(key.clone());
            } else {
                let cur = &current_files[key];
                let prev = &previous[key];
                if !Arc::ptr_eq(cur, prev) && cur != prev {
                    modified.push(key.clone());
                }
            }
        }
        for key in previous.keys() {
            if !current_files.contains_key(key) {
                removed.push(key.clone());
            }
        }
        added.sort();
        modified.sort();
        removed.sort();
        FileDelta {
            added,
            modified,
            removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drun_core::{CheckpointRef, Config};
    use std::time::Duration;

    fn file_map(pairs: &[(&str, &[u8])]) -> FileMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Arc::new(v.to_vec())))
            .collect()
    }

    fn new_session() -> Session {
        Session::new(Config::default().into()).unwrap()
    }

    #[test]
    fn compute_with_no_previous_reports_nothing() {
        let current = file_map(&[("a.txt", b"hello")]);
        let delta = FileDelta::compute(None, &current);
        assert_eq!(
            delta,
            FileDelta {
                added: vec![],
                modified: vec![],
                removed: vec![]
            }
        );
    }

    #[test]
    fn compute_detects_added_file() {
        let previous = file_map(&[]);
        let current = file_map(&[("new.txt", b"hello")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.added, vec!["new.txt".to_string()]);
        assert!(delta.modified.is_empty());
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn compute_detects_removed_file() {
        let previous = file_map(&[("gone.txt", b"hello")]);
        let current = file_map(&[]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.removed, vec!["gone.txt".to_string()]);
        assert!(delta.added.is_empty());
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_detects_modified_file_by_content() {
        let previous = file_map(&[("a.txt", b"old")]);
        let current = file_map(&[("a.txt", b"new")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(delta.modified, vec!["a.txt".to_string()]);
    }

    #[test]
    fn compute_ignores_unchanged_file_even_with_different_arc_allocation() {
        // Same bytes, but two separate Arc allocations (not Arc::ptr_eq) — content
        // equality must win over pointer inequality here.
        let previous = file_map(&[("a.txt", b"same")]);
        let current = file_map(&[("a.txt", b"same")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_ignores_unchanged_file_sharing_the_same_arc() {
        let shared = Arc::new(b"same".to_vec());
        let mut previous = FileMap::new();
        previous.insert("a.txt".to_string(), Arc::clone(&shared));
        let mut current = FileMap::new();
        current.insert("a.txt".to_string(), shared);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert!(delta.modified.is_empty());
    }

    #[test]
    fn compute_sorts_each_category() {
        let previous = file_map(&[("z_remove.txt", b"1"), ("m_remove.txt", b"1")]);
        let current = file_map(&[("z_add.txt", b"1"), ("a_add.txt", b"1")]);
        let delta = FileDelta::compute(Some(&previous), &current);
        assert_eq!(
            delta.added,
            vec!["a_add.txt".to_string(), "z_add.txt".to_string()]
        );
        assert_eq!(
            delta.removed,
            vec!["m_remove.txt".to_string(), "z_remove.txt".to_string()]
        );
    }

    #[test]
    fn session_summary_all_reflects_current_checkpoint_and_history_length() {
        let mut session = new_session();
        session.write_file("a.txt", b"hi".to_vec()).unwrap();
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(session)));

        let summaries = SessionSummary::all(&sessions);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, "s1");
        assert_eq!(summaries[0].checkpoint_id, 1);
        assert_eq!(summaries[0].checkpoint_count, 2);
        assert_eq!(summaries[0].parent_session_id, None);
    }

    #[test]
    fn session_summary_all_reports_parent_reference() {
        let mut session = new_session();
        session.parent = Some(CheckpointRef {
            session_id: "parent-session".to_string(),
            checkpoint_id: 3,
        });
        let mut sessions = HashMap::new();
        sessions.insert("child".to_string(), Arc::new(Mutex::new(session)));

        let summaries = SessionSummary::all(&sessions);
        assert_eq!(
            summaries[0].parent_session_id,
            Some("parent-session".to_string())
        );
        assert_eq!(summaries[0].parent_checkpoint_id, Some(3));
    }

    #[test]
    fn session_summary_all_recovers_from_a_poisoned_lock_instead_of_panicking() {
        let arc = Arc::new(Mutex::new(new_session()));
        let arc_for_panic = arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(arc.is_poisoned());

        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), arc);

        let summaries = SessionSummary::all(&sessions);
        assert_eq!(summaries.len(), 1);
    }

    #[test]
    fn session_state_compute_reports_zero_deltas_with_no_previous_files() {
        let session = new_session();
        let state = SessionState::compute("s1", &session, None, vec![]);
        assert_eq!(state.files_added_count, 0);
        assert_eq!(state.files_modified_count, 0);
        assert_eq!(state.files_removed_count, 0);
        assert!(state.committed_files.is_empty());
    }

    #[test]
    fn session_state_compute_counts_added_files_against_previous_snapshot() {
        let mut session = new_session();
        let previous_files = session.current().files.clone();
        session.write_file("a.txt", b"hi".to_vec()).unwrap();

        let state = SessionState::compute("s1", &session, Some(&previous_files), vec![]);
        assert_eq!(state.files_added_count, 1);
        assert_eq!(state.workspace_file_count, 1);
    }

    #[test]
    fn session_state_compute_passes_through_committed_files() {
        let session = new_session();
        let state = SessionState::compute(
            "s1",
            &session,
            None,
            vec!["a.txt".to_string(), "b.txt".to_string()],
        );
        assert_eq!(
            state.committed_files,
            vec!["a.txt".to_string(), "b.txt".to_string()]
        );
    }

    #[test]
    fn checkpoint_summary_history_treats_first_checkpoint_as_having_no_delta() {
        let session = new_session();
        let history = CheckpointSummary::history(&session);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].files_added_count, 0);
    }

    #[test]
    fn checkpoint_summary_history_diffs_against_the_prior_checkpoint() {
        let mut session = new_session();
        session.write_file("a.txt", b"hi".to_vec()).unwrap();
        session.write_file("b.txt", b"hi".to_vec()).unwrap();

        let history = CheckpointSummary::history(&session);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].files_added_count, 0);
        assert_eq!(history[1].files_added_count, 1); // a.txt
        assert_eq!(history[2].files_added_count, 1); // b.txt, not a.txt again
    }

    #[test]
    fn snapshot_entry_catalog_returns_empty_for_missing_directory() {
        let catalog = SnapshotEntry::catalog(Path::new("/nonexistent/drun-snapshots-test"));
        assert!(catalog.is_empty());
    }

    #[test]
    fn snapshot_entry_catalog_returns_empty_for_directory_with_no_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"hello").unwrap();
        let catalog = SnapshotEntry::catalog(dir.path());
        assert!(catalog.is_empty());
    }

    #[test]
    fn snapshot_entry_catalog_finds_a_real_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = new_session();
        session.set_label("checkpoint-1".to_string());
        session.write_file("a.txt", b"hi".to_vec()).unwrap();
        let snapshot_path = dir.path().join("session.drun");
        session.snapshot().write(&snapshot_path).unwrap();

        let catalog = SnapshotEntry::catalog(dir.path());
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].checkpoint_count, 2);
    }

    #[test]
    fn session_tree_node_forest_produces_a_single_root_for_an_unrelated_session() {
        let session = new_session();
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(session)));

        let forest = SessionTreeNode::forest(&sessions);
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].session_id, "s1");
        assert!(forest[0].checkpoints[0].forks.is_empty());
    }

    #[test]
    fn session_tree_node_forest_recovers_from_a_poisoned_lock_instead_of_panicking() {
        let arc = Arc::new(Mutex::new(new_session()));
        let arc_for_panic = arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(arc.is_poisoned());

        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), arc);

        let forest = SessionTreeNode::forest(&sessions);
        assert_eq!(forest.len(), 1);
    }

    #[test]
    fn session_tree_node_forest_nests_a_fork_under_its_parent() {
        let parent = new_session();
        let mut child = new_session();
        child.parent = Some(CheckpointRef {
            session_id: "parent".to_string(),
            checkpoint_id: 0,
        });
        let mut sessions = HashMap::new();
        sessions.insert("parent".to_string(), Arc::new(Mutex::new(parent)));
        sessions.insert("child".to_string(), Arc::new(Mutex::new(child)));

        let forest = SessionTreeNode::forest(&sessions);
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].session_id, "parent");
        assert_eq!(forest[0].checkpoints[0].forks.len(), 1);
        assert_eq!(forest[0].checkpoints[0].forks[0].session_id, "child");
    }

    #[test]
    fn session_tree_node_forest_treats_a_dangling_parent_reference_as_a_root() {
        let mut session = new_session();
        session.parent = Some(CheckpointRef {
            session_id: "deleted-parent".to_string(),
            checkpoint_id: 0,
        });
        let mut sessions = HashMap::new();
        sessions.insert("orphan".to_string(), Arc::new(Mutex::new(session)));

        let forest = SessionTreeNode::forest(&sessions);
        assert_eq!(forest.len(), 1);
        assert_eq!(forest[0].session_id, "orphan");
    }

    #[test]
    fn session_tree_node_reports_age_and_idle_seconds() {
        let mut session = new_session();
        session.created_at = Instant::now() - Duration::from_secs(300);
        session.last_activity = Instant::now() - Duration::from_secs(120);
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(session)));

        let forest = SessionTreeNode::forest(&sessions);
        assert!(forest[0].age_secs >= 300);
        assert!(forest[0].idle_secs >= 120 && forest[0].idle_secs < 300);
    }

    #[test]
    fn daemon_status_current_reports_session_count_and_config_limits() {
        let config = Config {
            max_sessions: Some(10),
            ..Config::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(new_session())));

        let status = DaemonStatus::current(&sessions, &config, Instant::now(), 7273, 7274);
        assert_eq!(status.session_count, 1);
        assert_eq!(status.max_sessions, Some(10));
        assert_eq!(status.mcp_port, 7273);
        assert_eq!(status.web_port, 7274);
        assert_eq!(status.pid, std::process::id());
    }

    #[test]
    fn memory_rss_bytes_returns_a_plausible_value_on_this_platform() {
        assert!(DaemonStatus::memory_rss_bytes().unwrap_or(1) > 0);
    }
}
