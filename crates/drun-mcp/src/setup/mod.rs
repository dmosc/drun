//! `drun-mcp setup` — a one-shot local web wizard for installing the pieces
//! `install.sh` can't safely automate: Homebrew, Ollama, the `drun chat`
//! CLI, and Tailscale.

mod components;

use axum::{
    Router,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};

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
        .route("/api/setup/open-terminal", post(handle_open_terminal))
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
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let body =
        serde_json::to_string(&components::current_status()).unwrap_or_else(|_| "null".into());
    (StatusCode::OK, headers, body).into_response()
}

async fn handle_open_terminal() -> Response {
    components::open_terminal();
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_string(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn handle_index_serves_the_embedded_html_with_no_store_cache_control() {
        let response = handle_index().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        assert_eq!(body_string(response).await, EMBEDDED_SETUP_HTML);
    }

    #[tokio::test]
    async fn handle_status_reports_every_component_and_its_commands() {
        let response = handle_status().await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("\"daemon\""));
        assert!(body.contains("\"homebrew\""));
        assert!(body.contains("\"ollama\""));
        assert!(body.contains("\"chat_cli\""));
        assert!(body.contains("\"tailscale\""));
        assert!(body.contains("\"commands\""));
        assert!(body.contains("\"install_ollama\""));
    }
}
