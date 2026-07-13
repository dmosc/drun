use crate::reaper::SessionMap;
use crate::state;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use drun_core::ConfigHandle;
use serde::Deserialize;
use std::time::Instant;

pub(crate) struct WebServer {
    sessions: SessionMap,
    port: u16,
    config: ConfigHandle,
    started_at: Instant,
}

impl WebServer {
    const EMBEDDED_INDEX_HTML: &'static str = include_str!("assets/index.html");

    pub(crate) fn new(
        sessions: SessionMap,
        port: u16,
        config: ConfigHandle,
        started_at: Instant,
    ) -> Self {
        Self {
            sessions,
            port,
            config,
            started_at,
        }
    }

    pub(crate) async fn serve(self) {
        let bind_address = format!("127.0.0.1:{}", self.port);
        match tokio::net::TcpListener::bind(&bind_address).await {
            Ok(listener) => {
                eprintln!("drun: web UI → http://{bind_address}");
                let router =
                    Self::build_router(self.sessions, self.config, self.port, self.started_at);
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
    ) -> Router {
        Router::new()
            .route("/", get(Self::handle_index))
            .route("/api/status", get(Self::handle_status))
            .route("/api/sessions/tree", get(Self::handle_session_tree))
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
            .with_state(AppState {
                sessions,
                config,
                mcp_port: crate::MCP_PORT,
                web_port,
                started_at,
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
        Self::json_response(&state::SessionTreeNode::forest(&sessions))
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
}

#[derive(Deserialize)]
struct DiffQueryParams {
    from: Option<usize>,
    to: Option<usize>,
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
            mcp_port: 7273,
            web_port: 7274,
            started_at: Instant::now(),
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
        assert!(body.contains("\"mcp_port\":7273"));
        assert!(body.contains("\"web_port\":7274"));
    }
}
