use crate::handler::DrunHandler;
use crate::live_output::LiveOutputRegistry;
use crate::state::file_delta::FileDelta;
use drun_core::{CheckpointRef, Session, Step};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq, Serialize)]
struct CheckpointTreeNode {
    checkpoint_id: usize,
    is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    steps: Vec<Step>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    file_count: usize,
    files_added_count: usize,
    files_modified_count: usize,
    files_removed_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    forks: Vec<SessionTreeNode>,
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
    running: bool,
    checkpoints: Vec<CheckpointTreeNode>,
}

impl SessionTreeNode {
    fn from_session(
        session_id: &str,
        session: &Session,
        children: &HashMap<(String, usize), Vec<(String, &Session)>>,
        live_output: &LiveOutputRegistry,
    ) -> SessionTreeNode {
        let current_id = session.current().id;
        let history = session.history();
        let checkpoints = history
            .iter()
            .enumerate()
            .map(|(index, checkpoint)| {
                let forks = children
                    .get(&(session_id.to_string(), checkpoint.id))
                    .map(|kids| {
                        kids.iter()
                            .map(|(id, s)| {
                                SessionTreeNode::from_session(id, s, children, live_output)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let previous_files = if index > 0 {
                    Some(&history[index - 1].files)
                } else {
                    None
                };
                let delta = FileDelta::compute(previous_files, &checkpoint.files);
                CheckpointTreeNode {
                    checkpoint_id: checkpoint.id,
                    is_current: checkpoint.id == current_id,
                    label: checkpoint.label.clone(),
                    tool: checkpoint.tool.clone(),
                    command: checkpoint.command.clone(),
                    description: checkpoint.description.clone(),
                    steps: checkpoint.steps.clone(),
                    exit_code: checkpoint.exit_code,
                    stdout_bytes: checkpoint.stdout.len(),
                    stderr_bytes: checkpoint.stderr.len(),
                    file_count: checkpoint.files.len(),
                    files_added_count: delta.added.len(),
                    files_modified_count: delta.modified.len(),
                    files_removed_count: delta.removed.len(),
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
            running: live_output.is_running(session_id),
            checkpoints,
        }
    }

    pub(crate) fn forest(
        sessions: &HashMap<String, Arc<Mutex<Session>>>,
        live_output: &LiveOutputRegistry,
    ) -> Vec<SessionTreeNode> {
        let mut locked_sessions: Vec<(String, std::sync::MutexGuard<Session>)> = Vec::new();
        let mut busy_ids: Vec<String> = Vec::new();
        for (id, arc) in sessions {
            match arc.try_lock() {
                Ok(guard) => locked_sessions.push((id.clone(), guard)),
                Err(std::sync::TryLockError::WouldBlock) => busy_ids.push(id.clone()),
                Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                    locked_sessions.push((id.clone(), DrunHandler::recover_poison(id, poisoned)))
                }
            }
        }

        let locked_ids: std::collections::HashSet<&str> =
            locked_sessions.iter().map(|(id, _)| id.as_str()).collect();

        let mut children: HashMap<(String, usize), Vec<(String, &Session)>> = HashMap::new();
        let mut roots: Vec<(&str, &Session)> = Vec::new();

        for (id, session) in &locked_sessions {
            let session: &Session = session;
            let parent_locked = session
                .parent
                .as_ref()
                .is_some_and(|r| locked_ids.contains(r.session_id.as_str()));
            if parent_locked {
                let r = session.parent.as_ref().unwrap();
                children
                    .entry((r.session_id.clone(), r.checkpoint_id))
                    .or_default()
                    .push((id.clone(), session));
            } else {
                roots.push((id.as_str(), session));
            }
        }

        let mut result: Vec<SessionTreeNode> = roots
            .into_iter()
            .map(|(id, session)| SessionTreeNode::from_session(id, session, &children, live_output))
            .collect();

        for id in busy_ids {
            result.push(SessionTreeNode {
                session_id: id,
                label: None,
                parent_session_id: None,
                parent_checkpoint_id: None,
                age_secs: 0,
                idle_secs: 0,
                running: true,
                checkpoints: Vec::new(),
            });
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drun_core::Config;
    use std::time::{Duration, Instant};

    fn new_session() -> Session {
        Session::new(Config::default().into()).unwrap()
    }

    #[test]
    fn session_tree_node_forest_reports_checkpoint_provenance_and_file_deltas() {
        let mut session = new_session();
        session
            .write_files(
                vec![("a.txt".to_string(), b"hi".to_vec())],
                "session_write_file",
                Some("add a.txt"),
            )
            .unwrap();
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(session)));

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());
        let checkpoint = &forest[0].checkpoints[1];
        assert_eq!(checkpoint.tool.as_deref(), Some("session_write_file"));
        assert_eq!(checkpoint.description.as_deref(), Some("add a.txt"));
        assert_eq!(checkpoint.files_added_count, 1);
    }

    #[test]
    fn session_tree_node_forest_produces_a_single_root_for_an_unrelated_session() {
        let session = new_session();
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(session)));

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());
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

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());
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

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());
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

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());
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

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());
        assert!(forest[0].age_secs >= 300);
        assert!(forest[0].idle_secs >= 120 && forest[0].idle_secs < 300);
    }

    #[test]
    fn session_tree_node_reports_running_when_a_command_is_in_flight() {
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), Arc::new(Mutex::new(new_session())));
        let live_output = LiveOutputRegistry::default();
        let guard = live_output.start("s1", "echo hi");

        let forest = SessionTreeNode::forest(&sessions, &live_output);
        assert!(forest[0].running);

        drop(guard);
        let forest = SessionTreeNode::forest(&sessions, &live_output);
        assert!(!forest[0].running);
    }

    #[test]
    fn session_tree_node_forest_does_not_block_on_a_busy_sessions_own_lock() {
        // Reproduces the real shape of an in-flight session_bash: the
        // session's own Mutex is held (not just a live_output entry) for the
        // call's duration. forest() must not block waiting for it — that
        // would stall /api/sessions/tree, and therefore the whole 2s UI
        // poll, for as long as the command runs.
        let arc = Arc::new(Mutex::new(new_session()));
        let mut sessions = HashMap::new();
        sessions.insert("s1".to_string(), arc.clone());
        let live_output = LiveOutputRegistry::default();

        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let hold_thread = std::thread::spawn(move || {
            let _guard = arc.lock().unwrap();
            let _ = release_rx.recv();
        });
        // Give the spawned thread time to actually acquire the lock before
        // this thread's forest() call races it.
        std::thread::sleep(Duration::from_millis(50));
        let _live_guard = live_output.start("s1", "sleep 30");

        let forest = SessionTreeNode::forest(&sessions, &live_output);

        release_tx.send(()).unwrap();
        hold_thread.join().unwrap();

        assert_eq!(forest.len(), 1, "the busy session must still appear");
        assert_eq!(forest[0].session_id, "s1");
        assert!(forest[0].running);
    }

    #[test]
    fn session_tree_node_forest_still_surfaces_a_fork_whose_parent_is_busy() {
        // A busy parent can't be walked to reach its children through the
        // usual checkpoint-forks nesting, so a busy parent must not make its
        // children vanish from the tree entirely.
        let parent_arc = Arc::new(Mutex::new(new_session()));
        let mut child = new_session();
        child.parent = Some(CheckpointRef {
            session_id: "parent".to_string(),
            checkpoint_id: 0,
        });
        let mut sessions = HashMap::new();
        sessions.insert("parent".to_string(), parent_arc.clone());
        sessions.insert("child".to_string(), Arc::new(Mutex::new(child)));

        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let hold_thread = std::thread::spawn(move || {
            let _guard = parent_arc.lock().unwrap();
            let _ = release_rx.recv();
        });
        std::thread::sleep(Duration::from_millis(50));

        let forest = SessionTreeNode::forest(&sessions, &LiveOutputRegistry::default());

        release_tx.send(()).unwrap();
        hold_thread.join().unwrap();

        assert!(
            forest.iter().any(|node| node.session_id == "child"),
            "child must still appear even though its parent is busy"
        );
    }
}
