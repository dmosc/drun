use crate::handler::{self, CloseSessionError};
use crate::live_output::{LiveEntry, LiveOutputRegistry};
use crate::reaper::SessionMap;
use crate::response::mime_type_for_extension;
use crate::state;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use drun_core::ConfigHandle;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub(crate) struct WebServer {
    sessions: SessionMap,
    port: u16,
    config: ConfigHandle,
    started_at: Instant,
    live_output: LiveOutputRegistry,
}

impl WebServer {
    const EMBEDDED_INDEX_HTML: &'static str = include_str!("assets/index.html");

    pub(crate) fn new(
        sessions: SessionMap,
        port: u16,
        config: ConfigHandle,
        started_at: Instant,
        live_output: LiveOutputRegistry,
    ) -> Self {
        Self {
            sessions,
            port,
            config,
            started_at,
            live_output,
        }
    }

    pub(crate) async fn serve(self) {
        let bind_address = format!("127.0.0.1:{}", self.port);
        match tokio::net::TcpListener::bind(&bind_address).await {
            Ok(listener) => {
                eprintln!("drun: web UI → http://{bind_address}");
                let router = Self::build_router(
                    self.sessions,
                    self.config,
                    self.port,
                    self.started_at,
                    self.live_output,
                );
                axum::serve(listener, router).await.ok();
            }
            Err(error) => {
                eprintln!("drun: web UI failed to bind on {bind_address}: {error}");
            }
        }
    }

    fn build_router(
        sessions: SessionMap,
        config: ConfigHandle,
        web_port: u16,
        started_at: Instant,
        live_output: LiveOutputRegistry,
    ) -> Router {
        Router::new()
            .route("/", get(Self::handle_index))
            .route("/api/status", get(Self::handle_status))
            .route("/api/sessions/tree", get(Self::handle_session_tree))
            .route(
                "/api/sessions/{session_id}/live",
                get(Self::handle_live_output),
            )
            .route(
                "/api/sessions/{session_id}/history",
                get(Self::handle_checkpoint_history),
            )
            .route(
                "/api/sessions/{session_id}/diff",
                get(Self::handle_checkpoint_diff),
            )
            .route(
                "/api/sessions/{session_id}/checkpoints/{checkpoint_id}/stdout",
                get(Self::handle_checkpoint_stdout),
            )
            .route(
                "/api/sessions/{session_id}/checkpoints/{checkpoint_id}/stderr",
                get(Self::handle_checkpoint_stderr),
            )
            .route(
                "/api/sessions/{session_id}/checkpoints/{checkpoint_id}/files",
                get(Self::handle_checkpoint_files),
            )
            .route(
                "/api/sessions/{session_id}/checkpoints/{checkpoint_id}/files/{*path}",
                get(Self::handle_checkpoint_file),
            )
            .route(
                "/api/sessions/{session_id}",
                axum::routing::delete(Self::handle_session_delete),
            )
            .with_state(AppState {
                sessions,
                config,
                mcp_port: crate::mcp_port(),
                web_port,
                started_at,
                live_output,
            })
    }

    async fn handle_status(State(app): State<AppState>) -> Response {
        let sessions = app.sessions.lock().unwrap();
        let config = app.config.get();
        Self::json_response(&state::DaemonStatus::current(
            &sessions,
            &config,
            app.started_at,
            app.mcp_port,
            app.web_port,
        ))
    }

    async fn handle_index() -> Response {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
        (StatusCode::OK, headers, Self::EMBEDDED_INDEX_HTML).into_response()
    }

    async fn handle_session_tree(State(app): State<AppState>) -> Response {
        let sessions = app.sessions.lock().unwrap();
        Self::json_response(&state::SessionTreeNode::forest(&sessions, &app.live_output))
    }

    async fn handle_live_output(
        State(app): State<AppState>,
        Path(session_id): Path<String>,
    ) -> Response {
        // Deliberately checks presence in the session map only, not
        // `with_session`'s try_lock — a session busy running a command is
        // exactly the case this endpoint exists to serve, not a 503.
        if !app.sessions.lock().unwrap().contains_key(&session_id) {
            return (
                StatusCode::NOT_FOUND,
                format!("session '{session_id}' not found"),
            )
                .into_response();
        }
        Self::json_response(&LiveOutput::from(app.live_output.snapshot(&session_id)))
    }

    async fn handle_checkpoint_history(
        State(app): State<AppState>,
        Path(session_id): Path<String>,
    ) -> Response {
        Self::with_session(&app.sessions, &session_id, |session| {
            Self::json_response(&state::CheckpointSummary::history(session))
        })
    }

    async fn handle_checkpoint_diff(
        State(app): State<AppState>,
        Path(session_id): Path<String>,
        Query(params): Query<DiffQueryParams>,
    ) -> Response {
        let from_id = params.from.unwrap_or(0);
        Self::with_session(&app.sessions, &session_id, move |session| {
            let to_id = params.to.unwrap_or(session.current().id);
            match session.diff(from_id, to_id) {
                Ok(diff) => (StatusCode::OK, diff).into_response(),
                Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
            }
        })
    }

    async fn handle_checkpoint_stdout(
        State(app): State<AppState>,
        Path((session_id, checkpoint_id)): Path<(String, usize)>,
    ) -> Response {
        Self::read_checkpoint_stream(&app.sessions, &session_id, checkpoint_id, |cp| {
            cp.stdout.clone()
        })
    }

    async fn handle_checkpoint_stderr(
        State(app): State<AppState>,
        Path((session_id, checkpoint_id)): Path<(String, usize)>,
    ) -> Response {
        Self::read_checkpoint_stream(&app.sessions, &session_id, checkpoint_id, |cp| {
            cp.stderr.clone()
        })
    }

    async fn handle_checkpoint_files(
        State(app): State<AppState>,
        Path((session_id, checkpoint_id)): Path<(String, usize)>,
    ) -> Response {
        Self::with_session(&app.sessions, &session_id, |session| {
            match session.history().get(checkpoint_id) {
                Some(checkpoint) => {
                    let mut files: Vec<FileEntry> = checkpoint
                        .files
                        .iter()
                        .map(|(path, bytes)| FileEntry {
                            path: path.clone(),
                            size_bytes: bytes.len(),
                        })
                        .collect();
                    files.sort_by(|a, b| a.path.cmp(&b.path));
                    Self::json_response(&files)
                }
                None => (
                    StatusCode::NOT_FOUND,
                    format!("checkpoint {checkpoint_id} not found"),
                )
                    .into_response(),
            }
        })
    }

    async fn handle_checkpoint_file(
        State(app): State<AppState>,
        Path((session_id, checkpoint_id, path)): Path<(String, usize, String)>,
    ) -> Response {
        Self::with_session(&app.sessions, &session_id, move |session| {
            let Some(checkpoint) = session.history().get(checkpoint_id) else {
                return (
                    StatusCode::NOT_FOUND,
                    format!("checkpoint {checkpoint_id} not found"),
                )
                    .into_response();
            };
            match checkpoint.files.get(&path) {
                Some(bytes) => Self::file_response(&path, bytes),
                None => (
                    StatusCode::NOT_FOUND,
                    format!("file '{path}' not found in checkpoint {checkpoint_id}"),
                )
                    .into_response(),
            }
        })
    }

    async fn handle_session_delete(
        State(app): State<AppState>,
        Path(session_id): Path<String>,
    ) -> Response {
        match handler::close_session(&app.sessions, &app.config, &session_id) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(CloseSessionError::NotFound) => (
                StatusCode::NOT_FOUND,
                format!("session '{session_id}' not found"),
            )
                .into_response(),
            Err(CloseSessionError::Io(error)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
            }
        }
    }

    fn file_response(path: &str, bytes: &[u8]) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(mime_type) = mime_type_for_extension(path) {
            headers.insert("content-type", HeaderValue::from_static(mime_type));
            return (StatusCode::OK, headers, bytes.to_vec()).into_response();
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                headers.insert(
                    "content-type",
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                );
                (StatusCode::OK, headers, text.to_string()).into_response()
            }
            Err(_) => {
                headers.insert(
                    "content-type",
                    HeaderValue::from_static("application/octet-stream"),
                );
                headers.insert("x-drun-binary", HeaderValue::from_static("true"));
                (StatusCode::OK, headers, bytes.to_vec()).into_response()
            }
        }
    }

    fn with_session(
        sessions: &SessionMap,
        session_id: &str,
        handler: impl FnOnce(&drun_core::Session) -> Response,
    ) -> Response {
        let session_arc = match sessions.lock().unwrap().get(session_id).cloned() {
            Some(arc) => arc,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    format!("session '{session_id}' not found"),
                )
                    .into_response();
            }
        };
        match session_arc.try_lock() {
            Ok(guard) => handler(&guard),
            Err(std::sync::TryLockError::WouldBlock) => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("session '{session_id}' is currently executing; retry shortly"),
            )
                .into_response(),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => handler(
                &crate::handler::DrunHandler::recover_poison(session_id, poisoned),
            ),
        }
    }

    fn read_checkpoint_stream(
        sessions: &SessionMap,
        session_id: &str,
        checkpoint_id: usize,
        extract: impl FnOnce(&drun_core::Checkpoint) -> String,
    ) -> Response {
        Self::with_session(sessions, session_id, |session| {
            match session.history().get(checkpoint_id) {
                Some(checkpoint) => (StatusCode::OK, extract(checkpoint)).into_response(),
                None => (
                    StatusCode::NOT_FOUND,
                    format!("checkpoint {checkpoint_id} not found"),
                )
                    .into_response(),
            }
        })
    }

    fn json_response(value: &impl serde::Serialize) -> Response {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let body = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
        (StatusCode::OK, headers, body).into_response()
    }
}

#[derive(Clone)]
struct AppState {
    sessions: SessionMap,
    config: ConfigHandle,
    mcp_port: u16,
    web_port: u16,
    started_at: Instant,
    live_output: LiveOutputRegistry,
}

#[derive(Deserialize)]
struct DiffQueryParams {
    from: Option<usize>,
    to: Option<usize>,
}

#[derive(Serialize)]
struct FileEntry {
    path: String,
    size_bytes: usize,
}

#[derive(Serialize)]
struct LiveOutput {
    running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    output: String,
}

impl From<Option<LiveEntry>> for LiveOutput {
    fn from(entry: Option<LiveEntry>) -> Self {
        match entry {
            Some(LiveEntry { command, output }) => LiveOutput {
                running: true,
                command: Some(command),
                output,
            },
            None => LiveOutput {
                running: false,
                command: None,
                output: String::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use drun_core::{Config, Session};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn session_map(entries: Vec<(&str, Session)>) -> SessionMap {
        let mut map = HashMap::new();
        for (id, session) in entries {
            map.insert(id.to_string(), Arc::new(Mutex::new(session)));
        }
        Arc::new(Mutex::new(map))
    }

    fn app_state(sessions: SessionMap) -> AppState {
        AppState {
            sessions,
            config: Config::default().into(),
            mcp_port: crate::DEFAULT_MCP_PORT,
            web_port: 7274,
            started_at: Instant::now(),
            live_output: LiveOutputRegistry::default(),
        }
    }

    async fn body_string(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn handle_index_serves_the_embedded_html_with_no_store_cache_control() {
        let response = WebServer::handle_index().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body = body_string(response).await;
        assert_eq!(body, WebServer::EMBEDDED_INDEX_HTML);
    }

    #[tokio::test]
    async fn handle_session_tree_returns_json_for_the_current_sessions() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_session_tree(State(app_state(sessions))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("s1"));
    }

    #[tokio::test]
    async fn handle_checkpoint_history_returns_404_for_an_unknown_session() {
        let sessions = session_map(vec![]);
        let response = WebServer::handle_checkpoint_history(
            State(app_state(sessions)),
            Path("missing".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_checkpoint_history_returns_json_history_on_success() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_checkpoint_history(
            State(app_state(sessions)),
            Path("s1".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("checkpoint_id"));
    }

    #[tokio::test]
    async fn handle_checkpoint_history_recovers_from_a_poisoned_lock_instead_of_staying_503_forever()
     {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let session_arc = sessions.lock().unwrap().get("s1").unwrap().clone();
        let arc_for_panic = session_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(session_arc.is_poisoned());

        let response = WebServer::handle_checkpoint_history(
            State(app_state(sessions)),
            Path("s1".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handle_checkpoint_diff_defaults_to_diffing_from_checkpoint_zero() {
        let mut session = Session::new(Config::default().into()).unwrap();
        session.write_file("a.txt", b"hi".to_vec()).unwrap();
        let sessions = session_map(vec![("s1", session)]);

        let response = WebServer::handle_checkpoint_diff(
            State(app_state(sessions)),
            Path("s1".to_string()),
            Query(DiffQueryParams {
                from: None,
                to: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("a.txt"));
    }

    #[tokio::test]
    async fn handle_checkpoint_diff_returns_400_for_an_invalid_checkpoint() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_checkpoint_diff(
            State(app_state(sessions)),
            Path("s1".to_string()),
            Query(DiffQueryParams {
                from: Some(99),
                to: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handle_checkpoint_stdout_returns_404_for_an_unknown_checkpoint() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_checkpoint_stdout(
            State(app_state(sessions)),
            Path(("s1".to_string(), 99)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_checkpoint_stdout_returns_the_checkpoints_stdout() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_checkpoint_stdout(
            State(app_state(sessions)),
            Path(("s1".to_string(), 0)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn handle_status_reports_session_count_and_ports() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_status(State(app_state(sessions))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("\"session_count\":1"));
        assert!(body.contains(&format!("\"mcp_port\":{}", crate::DEFAULT_MCP_PORT)));
        assert!(body.contains("\"web_port\":7274"));
    }

    #[tokio::test]
    async fn handle_checkpoint_files_lists_paths_sorted_with_sizes() {
        let mut session = Session::new(Config::default().into()).unwrap();
        session.write_file("z.txt", b"12345".to_vec()).unwrap();
        session.write_file("a.txt", b"hi".to_vec()).unwrap();
        let sessions = session_map(vec![("s1", session)]);

        let response = WebServer::handle_checkpoint_files(
            State(app_state(sessions)),
            Path(("s1".to_string(), 2)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert_eq!(
            body,
            r#"[{"path":"a.txt","size_bytes":2},{"path":"z.txt","size_bytes":5}]"#
        );
    }

    #[tokio::test]
    async fn handle_checkpoint_files_returns_404_for_an_unknown_checkpoint() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_checkpoint_files(
            State(app_state(sessions)),
            Path(("s1".to_string(), 99)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_checkpoint_file_returns_utf8_text_as_plain_text() {
        let mut session = Session::new(Config::default().into()).unwrap();
        session.write_file("notes.txt", b"hello".to_vec()).unwrap();
        let sessions = session_map(vec![("s1", session)]);

        let response = WebServer::handle_checkpoint_file(
            State(app_state(sessions)),
            Path(("s1".to_string(), 1, "notes.txt".to_string())),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/plain; charset=utf-8"
        );
        assert_eq!(body_string(response).await, "hello");
    }

    #[tokio::test]
    async fn handle_checkpoint_file_marks_non_utf8_content_as_binary() {
        let mut session = Session::new(Config::default().into()).unwrap();
        session
            .write_file("data.bin", vec![0xff, 0xfe, 0x00])
            .unwrap();
        let sessions = session_map(vec![("s1", session)]);

        let response = WebServer::handle_checkpoint_file(
            State(app_state(sessions)),
            Path(("s1".to_string(), 1, "data.bin".to_string())),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-drun-binary").unwrap(), "true");
    }

    #[tokio::test]
    async fn handle_checkpoint_file_returns_404_for_an_unknown_path() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response = WebServer::handle_checkpoint_file(
            State(app_state(sessions)),
            Path(("s1".to_string(), 0, "missing.txt".to_string())),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_session_delete_removes_the_session_from_the_map() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let state = app_state(sessions.clone());

        let response = WebServer::handle_session_delete(State(state), Path("s1".to_string())).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(!sessions.lock().unwrap().contains_key("s1"));
    }

    #[tokio::test]
    async fn handle_session_delete_returns_404_for_an_unknown_session() {
        let sessions = session_map(vec![]);
        let response = WebServer::handle_session_delete(
            State(app_state(sessions)),
            Path("missing".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_live_output_reports_not_running_for_an_idle_session() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let response =
            WebServer::handle_live_output(State(app_state(sessions)), Path("s1".to_string())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert_eq!(body, r#"{"running":false,"output":""}"#);
    }

    #[tokio::test]
    async fn handle_live_output_returns_404_for_an_unknown_session() {
        let response = WebServer::handle_live_output(
            State(app_state(session_map(vec![]))),
            Path("missing".to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_live_output_streams_the_command_and_output_so_far() {
        let sessions = session_map(vec![(
            "s1",
            Session::new(Config::default().into()).unwrap(),
        )]);
        let state = app_state(sessions);
        let guard = state.live_output.start("s1", "echo hi");
        guard.append("hello");

        let response = WebServer::handle_live_output(State(state), Path("s1".to_string())).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert_eq!(
            body,
            r#"{"running":true,"command":"echo hi","output":"hello\n"}"#
        );
    }
}
