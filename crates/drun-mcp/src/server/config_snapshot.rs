use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::state::{SessionState, SnapshotEntry};
use crate::tools::{SessionGetEnv, SessionRestore, SessionSnapshotTool};
use drun_core::{Session, SessionSnapshot};
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::path::{Path, PathBuf};
use uuid::Uuid;

impl DrunHandler {
    pub(super) fn handle_list_snapshots(&self) -> Result<CallToolResult, CallToolError> {
        Ok(ResponseBuilder::json(&SnapshotEntry::catalog(
            &self.config.get().snapshots_dir,
        )))
    }

    pub(super) fn handle_get_system_instructions(&self) -> Result<CallToolResult, CallToolError> {
        Ok(ResponseBuilder::text(
            crate::instructions::SYSTEM_INSTRUCTIONS,
        ))
    }

    pub(super) fn handle_get_config(&self) -> Result<CallToolResult, CallToolError> {
        let config = self.config.get();
        Ok(ResponseBuilder::text(
            serde_json::json!({
                "domain_allowlist": config.domain_allowlist,
                "mount_allowlist": config
                    .mount_allowlist
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
                "mount_overlay_paths": config.mount_overlay_paths,
                "env_allowlist": config.env_allowlist,
                "bash_command_denylist": config.bash_command_denylist,
                "bash_command_allowlist": config.bash_command_allowlist,
                "snapshots_dir": config.snapshots_dir.display().to_string(),
                "max_workspace_mb": config.max_workspace_mb,
                "max_sessions": config.max_sessions,
                "max_checkpoints": config.max_checkpoints,
                "session_idle_timeout_secs": config.session_idle_timeout_secs,
                "bash_timeout_ms": config.bash_timeout_ms,
                "fetch_timeout_ms": config.fetch_timeout_ms,
                "connect_timeout_ms": config.connect_timeout_ms,
                "package_install_enabled": config.package_install_enabled,
                "package_install_timeout_ms": config.package_install_timeout_ms,
            })
            .to_string(),
        ))
    }

    pub(super) fn handle_session_snapshot(
        &self,
        connection_id: &str,
        t: SessionSnapshotTool,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        let snapshots_dir = self.config.get().snapshots_dir;
        let output_path = match t.path {
            Some(p) => Self::path_confined_to_root(
                PathBuf::from(p),
                &snapshots_dir,
                DrunError::snapshot_denied,
            )?,
            None => snapshots_dir.join(format!("{session_id}.drun")),
        };
        if let Some(parent_dir) = output_path.parent() {
            std::fs::create_dir_all(parent_dir)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
        }
        self.with_session(&session_id, |session| {
            session
                .snapshot()
                .write(&output_path)
                .map_err(|e| DrunError::internal(e).into_tool_err())?;
            Ok(ResponseBuilder::text(
                serde_json::json!({
                    "snapshot_path": output_path.to_string_lossy(),
                })
                .to_string(),
            ))
        })
    }

    pub(super) fn handle_session_get_env(
        &self,
        connection_id: &str,
        t: SessionGetEnv,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        self.resolve_session(&session_id)?;
        if !self.config.get().env_allowlist.contains(&t.name) {
            return Err(DrunError::env_var_denied(&t.name).into_tool_err());
        }
        let value = std::env::var(&t.name).unwrap_or_default();
        Ok(ResponseBuilder::text(
            serde_json::json!({ "name": t.name, "value": value }).to_string(),
        ))
    }

    pub(super) fn handle_session_restore(
        &self,
        connection_id: &str,
        t: SessionRestore,
    ) -> Result<CallToolResult, CallToolError> {
        let bytes = std::fs::read(&t.path).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let snapshot =
            SessionSnapshot::decode(&bytes).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let restored = Session::from_snapshot(self.config.clone(), snapshot)
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let session_id = Uuid::new_v4().to_string();
        let state = SessionState::compute(&session_id, &restored, None, vec![]);
        self.insert_session(session_id.clone(), restored)
            .map_err(|max| DrunError::session_limit_reached(max).into_tool_err())?;
        self.current_sessions.set(connection_id, session_id);
        Ok(ResponseBuilder::json(&state))
    }

    fn path_confined_to_root(
        path: PathBuf,
        root: &Path,
        denied: impl Fn(&str, &str) -> DrunError,
    ) -> Result<PathBuf, CallToolError> {
        if path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(denied(&path.display().to_string(), "path must not contain '..'").into());
        }
        if !path.starts_with(root) {
            return Err(denied(&path.display().to_string(), &root.display().to_string()).into());
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use drun_core::Config;

    #[test]
    fn get_system_instructions_returns_the_full_guide() {
        let handler = DrunHandler::new(Config::default());
        let result = handler.handle_get_system_instructions().unwrap();
        assert_eq!(
            result_text(&result),
            crate::instructions::SYSTEM_INSTRUCTIONS
        );
    }

    #[test]
    fn session_restore_rejects_once_max_sessions_is_reached() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            max_sessions: Some(1),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "original");
        let snapshot_path = dir.path().join("original.drun");
        {
            let sessions = handler.sessions.lock().unwrap();
            let session = sessions.get("original").unwrap().lock().unwrap();
            session.snapshot().write(&snapshot_path).unwrap();
        }

        let err = handler
            .handle_session_restore(
                CLIENT,
                SessionRestore {
                    path: snapshot_path.to_string_lossy().into_owned(),
                },
            )
            .unwrap_err();

        assert!(err.to_string().contains("session_limit_reached"));
        assert_eq!(handler.sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn list_snapshots_returns_an_empty_catalog_for_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshots_dir: dir.path().join("does-not-exist"),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let result = handler.handle_list_snapshots().unwrap();
        assert_eq!(result_json(&result), serde_json::json!([]));
    }

    #[test]
    fn get_config_reports_the_configured_allowlists() {
        let config = Config {
            domain_allowlist: vec!["pypi.org".to_string()],
            mount_allowlist: vec![PathBuf::from("/home/user/project")],
            env_allowlist: vec!["API_KEY".to_string()],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let result = handler.handle_get_config().unwrap();
        let json = result_json(&result);
        assert_eq!(json["domain_allowlist"], serde_json::json!(["pypi.org"]));
        assert_eq!(
            json["mount_allowlist"],
            serde_json::json!(["/home/user/project"])
        );
        assert_eq!(json["env_allowlist"], serde_json::json!(["API_KEY"]));
    }

    #[test]
    fn get_config_reports_resource_limits() {
        let handler = DrunHandler::new(Config::default());
        let result = handler.handle_get_config().unwrap();
        let json = result_json(&result);
        assert_eq!(json["max_workspace_mb"], 512);
        assert_eq!(json["max_sessions"], 50);
        assert_eq!(json["max_checkpoints"], 200);
        assert_eq!(json["session_idle_timeout_secs"], 3600);
    }

    #[test]
    fn session_snapshot_rejects_a_path_containing_dotdot() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_snapshot(
                CLIENT,
                SessionSnapshotTool {
                    path: Some("../escape.drun".to_string()),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("snapshot_denied"));
    }

    #[test]
    fn session_snapshot_rejects_a_path_outside_the_snapshots_dir() {
        let config = Config {
            snapshots_dir: PathBuf::from("drun-snapshots"),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_snapshot(
                CLIENT,
                SessionSnapshotTool {
                    path: Some("/tmp/somewhere-else.drun".to_string()),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("snapshot_denied"));
    }

    #[test]
    fn session_snapshot_writes_under_the_default_snapshots_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshots_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");

        handler
            .handle_session_snapshot(CLIENT, SessionSnapshotTool { path: None })
            .unwrap();

        assert!(dir.path().join("s1.drun").exists());
    }

    #[test]
    fn session_snapshot_writes_to_an_explicit_path_under_snapshots_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshots_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");

        handler
            .handle_session_snapshot(
                CLIENT,
                SessionSnapshotTool {
                    path: Some(
                        dir.path()
                            .join("custom.drun")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                },
            )
            .unwrap();
        assert!(dir.path().join("custom.drun").exists());
    }

    #[test]
    fn session_get_env_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_get_env(
                CLIENT,
                SessionGetEnv {
                    name: "PATH".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[test]
    fn session_get_env_rejects_names_outside_the_allowlist() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_get_env(
                CLIENT,
                SessionGetEnv {
                    name: "SECRET".to_string(),
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("env_var_denied"));
    }

    #[test]
    fn session_get_env_returns_empty_string_for_an_unset_allowlisted_variable() {
        let config = Config {
            env_allowlist: vec!["DRUN_TEST_VAR_NOT_SET".to_string()],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");

        let result = handler
            .handle_session_get_env(
                CLIENT,
                SessionGetEnv {
                    name: "DRUN_TEST_VAR_NOT_SET".to_string(),
                },
            )
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["value"], "");
    }

    #[test]
    fn session_restore_recreates_a_session_from_a_snapshot_file() {
        let dir = tempfile::tempdir().unwrap();
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "original");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("original").unwrap().lock().unwrap();
            session
                .write_files(
                    vec![("a.txt".to_string(), b"hi".to_vec())],
                    "session_write_file",
                    None,
                )
                .unwrap();
        }
        let snapshot_path = dir.path().join("original.drun");
        {
            let sessions = handler.sessions.lock().unwrap();
            let session = sessions.get("original").unwrap().lock().unwrap();
            session.snapshot().write(&snapshot_path).unwrap();
        }

        let result = handler
            .handle_session_restore(
                CLIENT,
                SessionRestore {
                    path: snapshot_path.to_string_lossy().into_owned(),
                },
            )
            .unwrap();
        assert_eq!(result_json(&result)["workspace_file_count"], 1);
        assert_eq!(handler.sessions.lock().unwrap().len(), 2);
    }

    #[test]
    fn session_restore_makes_the_restored_session_current() {
        let dir = tempfile::tempdir().unwrap();
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "original");
        let snapshot_path = dir.path().join("original.drun");
        {
            let sessions = handler.sessions.lock().unwrap();
            let session = sessions.get("original").unwrap().lock().unwrap();
            session.snapshot().write(&snapshot_path).unwrap();
        }

        let result = handler
            .handle_session_restore(
                CLIENT,
                SessionRestore {
                    path: snapshot_path.to_string_lossy().into_owned(),
                },
            )
            .unwrap();
        let restored_id = result_json(&result)["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(handler.current_sessions.get(CLIENT), Some(restored_id));
    }

    #[test]
    fn session_restore_returns_an_error_for_a_missing_file() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_restore(
                CLIENT,
                SessionRestore {
                    path: "/nonexistent/path.drun".to_string(),
                },
            )
            .unwrap_err();
        assert!(!err.to_string().is_empty());
    }
}
