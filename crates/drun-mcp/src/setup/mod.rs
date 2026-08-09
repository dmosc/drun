//! `drun-mcp setup` — a one-shot local web wizard that walks a non-engineer
//! through the pieces `install.sh` can't safely automate itself: installing
//! Ollama, the `drun chat` CLI, and Tailscale, then wiring Tailscale up for
//! remote access. Every action it can run is also shown as a plain shell
//! command, so a stuck step (e.g. one that wants a sudo password, which this
//! process never feeds it — see `process::JobRegistry`) always has a
//! copy-paste fallback into the user's own terminal.

mod components;
mod process;

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use components::JobLookupError;
use process::JobRegistry;

const EMBEDDED_SETUP_HTML: &str = include_str!("../assets/setup.html");
const DEFAULT_PORT: u16 = 7275;

pub(crate) async fn run() {
    let port = std::env::var("DRUN_SETUP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let bind_address = format!("127.0.0.1:{port}");

    let listener = match tokio::net::TcpListener::bind(&bind_address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("drun: setup wizard failed to bind on {bind_address}: {error}");
            eprintln!("drun: set DRUN_SETUP_PORT to use a different port and try again.");
            return;
        }
    };

    let url = format!("http://{bind_address}");
    eprintln!("drun: setup wizard listening at {url}");
    open_browser(&url);

    axum::serve(listener, build_router()).await.ok();
}

fn open_browser(url: &str) {
    let opener = if std::env::consts::OS == "macos" {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn build_router() -> Router {
    Router::new()
        .route("/", get(handle_index))
        .route("/api/setup/status", get(handle_status))
        .route(
            "/api/setup/jobs/{job_id}",
            get(handle_job_output).post(handle_run_job),
        )
        .with_state(AppState::default())
}

#[derive(Clone)]
struct AppState {
    jobs: JobRegistry,
    /// Job id + configured web port -> the shell command to run. A function
    /// pointer (not a hardcoded call to `components::command_for`) so tests
    /// can substitute a harmless synthetic mapping instead of the real one —
    /// `command_for`'s real job ids run actual `brew`/`tailscale`/`ollama`
    /// commands, which must never fire as a side effect of `cargo test` on a
    /// machine that happens to have them installed.
    resolve_command: fn(&str, u16) -> Result<String, JobLookupError>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            jobs: JobRegistry::default(),
            resolve_command: components::command_for,
        }
    }
}

async fn handle_index() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    (StatusCode::OK, headers, EMBEDDED_SETUP_HTML).into_response()
}

async fn handle_status() -> Response {
    json_response(&components::current_status())
}

async fn handle_run_job(State(app): State<AppState>, Path(job_id): Path<String>) -> Response {
    let web_port = drun_core::Config::load().web_port.unwrap_or(7274);
    let command = match (app.resolve_command)(&job_id, web_port) {
        Ok(command) => command,
        Err(JobLookupError::Unknown) => {
            return (StatusCode::NOT_FOUND, format!("unknown job '{job_id}'")).into_response();
        }
        Err(JobLookupError::Unavailable(reason)) => {
            return (StatusCode::CONFLICT, reason).into_response();
        }
    };
    if !app.jobs.start(&job_id, &command) {
        return (StatusCode::CONFLICT, "job already running").into_response();
    }
    StatusCode::ACCEPTED.into_response()
}

async fn handle_job_output(State(app): State<AppState>, Path(job_id): Path<String>) -> Response {
    match app.jobs.snapshot(&job_id) {
        Some(state) => json_response(&state),
        None => json_response(&process::JobState {
            command: String::new(),
            output: String::new(),
            running: false,
            exit_code: None,
        }),
    }
}

fn json_response(value: &impl serde::Serialize) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let body = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    (StatusCode::OK, headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_string(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Never touches `components::command_for` — its real job ids run actual
    /// `brew`/`tailscale`/`ollama` commands, which handler tests must never
    /// execute for real. `known_job` maps to a harmless `echo`; every other
    /// id behaves like the real resolver's unknown-job case.
    fn test_command_for(job_id: &str, _web_port: u16) -> Result<String, JobLookupError> {
        match job_id {
            "known_job" => Ok("echo test output".to_string()),
            "blocked_job" => Err(JobLookupError::Unavailable(
                "not available here".to_string(),
            )),
            _ => Err(JobLookupError::Unknown),
        }
    }

    fn test_app_state() -> AppState {
        AppState {
            jobs: JobRegistry::default(),
            resolve_command: test_command_for,
        }
    }

    #[tokio::test]
    async fn handle_index_serves_the_embedded_html_with_no_store_cache_control() {
        let response = handle_index().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        assert_eq!(body_string(response).await, EMBEDDED_SETUP_HTML);
    }

    #[tokio::test]
    async fn handle_status_reports_json_for_every_component() {
        let response = handle_status().await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("\"daemon\""));
        assert!(body.contains("\"ollama\""));
        assert!(body.contains("\"chat_cli\""));
        assert!(body.contains("\"tailscale\""));
    }

    #[tokio::test]
    async fn handle_run_job_returns_404_for_an_unknown_job() {
        let response =
            handle_run_job(State(test_app_state()), Path("not_a_real_job".to_string())).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn handle_run_job_returns_409_for_a_job_unavailable_on_this_platform() {
        let response =
            handle_run_job(State(test_app_state()), Path("blocked_job".to_string())).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body_string(response).await, "not available here");
    }

    #[tokio::test]
    async fn handle_run_job_starts_a_known_job_and_output_becomes_pollable() {
        let app = test_app_state();
        let response = handle_run_job(State(app.clone()), Path("known_job".to_string())).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let output = handle_job_output(State(app), Path("known_job".to_string())).await;
        assert_eq!(output.status(), StatusCode::OK);
        assert!(
            body_string(output)
                .await
                .contains("\"command\":\"echo test output\"")
        );
    }

    #[tokio::test]
    async fn handle_run_job_rejects_a_second_start_while_the_first_is_still_running() {
        let app = test_app_state();
        let first = handle_run_job(State(app.clone()), Path("known_job".to_string())).await;
        assert_eq!(first.status(), StatusCode::ACCEPTED);

        let second = handle_run_job(State(app), Path("known_job".to_string())).await;
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn handle_job_output_for_a_job_that_never_ran_reports_idle() {
        let app = test_app_state();
        let response = handle_job_output(State(app), Path("known_job".to_string())).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_string(response).await,
            r#"{"command":"","output":"","running":false,"exit_code":null}"#
        );
    }
}
