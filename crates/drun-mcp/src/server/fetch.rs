use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::tools::SessionFetch;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::time::Duration;

impl DrunHandler {
    pub(super) async fn handle_session_fetch(
        &self,
        connection_id: &str,
        t: SessionFetch,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        self.resolve_session(&session_id)?;
        let config = self.config.get();
        let url_is_allowed = Self::host_from_url(&t.url).is_some_and(|h| config.domain_allowed(&h));
        if !url_is_allowed {
            return Err(DrunError::fetch_denied(&t.url).into_tool_err());
        }

        let method = t.method.as_deref().unwrap_or("GET").to_uppercase();
        let parsed_method = method.parse::<reqwest::Method>().map_err(|_| {
            DrunError::internal(format!("invalid HTTP method: {method}")).into_tool_err()
        })?;

        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .timeout(Duration::from_millis(config.fetch_timeout_ms));
        let client = builder
            .build()
            .map_err(|e| DrunError::internal(e).into_tool_err())?;

        let mut req = client.request(parsed_method, &t.url);
        if let Some(headers) = t.headers {
            for header in headers {
                req = req.header(header.name, header.value);
            }
        }
        if let Some(body) = t.body {
            req = req.body(body);
        }

        let mut response = req
            .send()
            .await
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let max_body = config
            .max_workspace_mb
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(256 * 1024 * 1024);
        let mut body_bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| DrunError::internal(e).into_tool_err())?
        {
            body_bytes.extend_from_slice(&chunk);
            if body_bytes.len() as u64 > max_body {
                return Err(DrunError::internal(format!(
                    "response body exceeds the {} MB limit; use a smaller download \
                     or raise max_workspace_mb in server config",
                    max_body / 1024 / 1024
                ))
                .into_tool_err());
            }
        }

        let save_path = t
            .save_to
            .unwrap_or_else(|| Self::download_path_from_url(&t.url));
        let bytes_len = body_bytes.len();
        self.with_session_mut(&session_id, |session| {
            session
                .write_file(&save_path, body_bytes.to_vec(), None)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::text(
                serde_json::json!({
                    "status": status,
                    "bytes": bytes_len,
                    "content_type": content_type,
                    "saved_to": save_path,
                })
                .to_string(),
            ))
        })
    }

    fn host_from_url(url: &str) -> Option<String> {
        let s = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let authority = s.split('/').next().filter(|h| !h.is_empty())?;
        let host = if authority.starts_with('[') {
            // IPv6: "[::1]" or "[::1]:port" — extract up to and including ']'
            let end = authority
                .find(']')
                .map(|i| i + 1)
                .unwrap_or(authority.len());
            authority[..end].to_string()
        } else {
            authority.split(':').next()?.to_string()
        };
        Some(host)
    }

    fn download_path_from_url(url: &str) -> String {
        let without_query = url.split('?').next().unwrap_or(url).trim_end_matches('/');
        let name = without_query
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("fetch");
        format!("downloads/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use crate::tools::HttpHeader;
    use drun_core::Config;

    #[test]
    fn host_from_url_extracts_https_host() {
        assert_eq!(
            DrunHandler::host_from_url("https://pypi.org/simple/requests/"),
            Some("pypi.org".to_string())
        );
    }

    #[test]
    fn host_from_url_extracts_http_host() {
        assert_eq!(
            DrunHandler::host_from_url("http://example.com"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn host_from_url_strips_port() {
        assert_eq!(
            DrunHandler::host_from_url("https://example.com:8080/path"),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn host_from_url_rejects_missing_scheme() {
        assert_eq!(DrunHandler::host_from_url("example.com/path"), None);
    }

    #[test]
    fn host_from_url_rejects_unsupported_scheme() {
        assert_eq!(DrunHandler::host_from_url("ftp://example.com/foo"), None);
    }

    #[test]
    fn host_from_url_rejects_empty_authority() {
        assert_eq!(DrunHandler::host_from_url("https:///path"), None);
    }

    #[test]
    fn host_from_url_handles_ipv6_with_port() {
        assert_eq!(
            DrunHandler::host_from_url("https://[::1]:8080/path"),
            Some("[::1]".to_string())
        );
    }

    #[test]
    fn host_from_url_handles_ipv6_without_port() {
        assert_eq!(
            DrunHandler::host_from_url("https://[::1]/path"),
            Some("[::1]".to_string())
        );
    }

    #[test]
    fn download_path_from_url_uses_last_path_segment() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/path/to/file.tar.gz"),
            "downloads/file.tar.gz"
        );
    }

    #[test]
    fn download_path_from_url_strips_query_string() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/file.zip?token=abc"),
            "downloads/file.zip"
        );
    }

    #[test]
    fn download_path_from_url_strips_trailing_slash() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/dir/"),
            "downloads/dir"
        );
    }

    #[test]
    fn download_path_from_url_falls_back_to_fetch_for_empty_path() {
        assert_eq!(
            DrunHandler::download_path_from_url("https://example.com/"),
            "downloads/example.com"
        );
        assert_eq!(DrunHandler::download_path_from_url(""), "downloads/fetch");
    }

    fn fetch_test_config(mock_uri: &str) -> Config {
        Config {
            domain_allowlist: vec![DrunHandler::host_from_url(mock_uri).unwrap()],
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn session_fetch_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: "https://pypi.org/simple/".to_string(),
                    method: None,
                    headers: None,
                    body: None,
                    save_to: None,
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[tokio::test]
    async fn session_fetch_denies_urls_outside_the_domain_allowlist() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: "https://evil.example.com/data".to_string(),
                    method: None,
                    headers: None,
                    body: None,
                    save_to: None,
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fetch_denied"));
    }

    #[tokio::test]
    async fn session_fetch_saves_the_response_body_under_the_default_download_path() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"ok":true}"#, "application/json"),
            )
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: format!("{}/data.json", mock_server.uri()),
                    method: None,
                    headers: None,
                    body: None,
                    save_to: None,
                },
            )
            .await
            .unwrap();

        let json = result_json(&result);
        assert_eq!(json["status"], 200);
        assert_eq!(json["content_type"], "application/json");
        assert_eq!(json["saved_to"], "downloads/data.json");

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        let saved = session.current().files.get("downloads/data.json").unwrap();
        assert_eq!(saved.as_slice(), br#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn session_fetch_honors_an_explicit_save_to_path_and_method() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/submit"))
            .respond_with(ResponseTemplate::new(201).set_body_string("created"))
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: format!("{}/submit", mock_server.uri()),
                    method: Some("post".to_string()),
                    headers: None,
                    body: Some("payload".to_string()),
                    save_to: Some("out/response.txt".to_string()),
                },
            )
            .await
            .unwrap();

        let json = result_json(&result);
        assert_eq!(json["status"], 201);
        assert_eq!(json["saved_to"], "out/response.txt");

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(
            session
                .current()
                .files
                .get("out/response.txt")
                .unwrap()
                .as_slice(),
            b"created"
        );
    }

    #[tokio::test]
    async fn session_fetch_rejects_an_invalid_http_method() {
        use wiremock::MockServer;

        // No Mock is registered: an invalid method token must be rejected
        // before any request reaches the (local, offline) mock server.
        let mock_server = MockServer::start().await;
        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: mock_server.uri(),
                    method: Some("IN VALID".to_string()),
                    headers: None,
                    body: None,
                    save_to: None,
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid HTTP method"));
    }

    #[tokio::test]
    async fn session_fetch_rejects_a_response_body_over_the_configured_limit() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 2048]))
            .mount(&mock_server)
            .await;

        let mut config = fetch_test_config(&mock_server.uri());
        config.max_workspace_mb = Some(0);
        let handler = DrunHandler::new(config);
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: mock_server.uri(),
                    method: None,
                    headers: None,
                    body: None,
                    save_to: None,
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[tokio::test]
    async fn session_fetch_forwards_custom_headers_to_the_request() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("x-api-key", "secret"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    url: mock_server.uri(),
                    method: None,
                    headers: Some(vec![HttpHeader {
                        name: "x-api-key".to_string(),
                        value: "secret".to_string(),
                    }]),
                    body: None,
                    save_to: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result_json(&result)["status"], 200);
    }
}
