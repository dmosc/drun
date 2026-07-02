use crate::reaper::SessionMap;
use crate::state;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

static EMBEDDED_INDEX_HTML: &str = include_str!("assets/index.html");

pub(crate) struct WebServer {
    sessions: SessionMap,
    port: u16,
}

impl WebServer {
    pub(crate) fn new(sessions: SessionMap, port: u16) -> Self {
        Self { sessions, port }
    }

    pub(crate) async fn serve(self) {
        let bind_address = format!("127.0.0.1:{}", self.port);
        match tokio::net::TcpListener::bind(&bind_address).await {
            Ok(listener) => {
                eprintln!("drun: web UI → http://{bind_address}");
                axum::serve(listener, build_router(self.sessions))
                    .await
                    .ok();
            }
            Err(error) => {
                eprintln!("drun: web UI failed to bind on {bind_address}: {error}");
            }
        }
    }
}

#[derive(Clone)]
struct AppState {
    sessions: SessionMap,
}

fn build_router(sessions: SessionMap) -> Router {
    Router::new()
        .route("/", get(handle_index))
        .route("/api/sessions/tree", get(handle_session_tree))
        .route(
            "/api/sessions/{session_id}/history",
            get(handle_checkpoint_history),
        )
        .route(
            "/api/sessions/{session_id}/diff",
            get(handle_checkpoint_diff),
        )
        .route(
            "/api/sessions/{session_id}/checkpoints/{checkpoint_id}/stdout",
            get(handle_checkpoint_stdout),
        )
        .route(
            "/api/sessions/{session_id}/checkpoints/{checkpoint_id}/stderr",
            get(handle_checkpoint_stderr),
        )
        .with_state(AppState { sessions })
}

async fn handle_index() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    (StatusCode::OK, headers, EMBEDDED_INDEX_HTML).into_response()
}

async fn handle_session_tree(State(app): State<AppState>) -> Response {
    let sessions = app.sessions.lock().unwrap();
    json_response(state::build_session_tree(&sessions))
}

async fn handle_checkpoint_history(
    State(app): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    with_session(&app.sessions, &session_id, |session| {
        json_response(state::build_checkpoint_history(session))
    })
}

#[derive(Deserialize)]
struct DiffQueryParams {
    from: Option<usize>,
    to: Option<usize>,
}

async fn handle_checkpoint_diff(
    State(app): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<DiffQueryParams>,
) -> Response {
    let from_id = params.from.unwrap_or(0);
    with_session(&app.sessions, &session_id, move |session| {
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
    read_checkpoint_stream(&app.sessions, &session_id, checkpoint_id, |cp| {
        cp.stdout.clone()
    })
}

async fn handle_checkpoint_stderr(
    State(app): State<AppState>,
    Path((session_id, checkpoint_id)): Path<(String, usize)>,
) -> Response {
    read_checkpoint_stream(&app.sessions, &session_id, checkpoint_id, |cp| {
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
    handler(&session_arc.lock().unwrap())
}

fn read_checkpoint_stream(
    sessions: &SessionMap,
    session_id: &str,
    checkpoint_id: usize,
    extract: impl FnOnce(&drun_core::Checkpoint) -> String,
) -> Response {
    with_session(sessions, session_id, |session| {
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

fn json_response(json: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    (StatusCode::OK, headers, json).into_response()
}
