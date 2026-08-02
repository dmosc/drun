use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::tools::SessionFetch;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use scraper::{Html, Selector};
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;
use url::Url;

impl DrunHandler {
    pub(super) async fn handle_session_fetch(
        &self,
        connection_id: &str,
        t: SessionFetch,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        self.resolve_session(&session_id)?;
        let config = self.config.get();

        let url = match Url::parse(&t.url) {
            Ok(url)
                if matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some_and(|h| config.domain_allowed(h)) =>
            {
                url
            }
            _ => return Err(DrunError::fetch_denied(&t.url).into_tool_err()),
        };

        let method = t.method.as_deref().unwrap_or("GET").to_uppercase();
        let parsed_method = method.parse::<reqwest::Method>().map_err(|_| {
            DrunError::internal(format!("invalid HTTP method: {method}")).into_tool_err()
        })?;

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .timeout(Duration::from_millis(config.fetch_timeout_ms))
            .build()
            .map_err(|e| DrunError::internal(e).into_tool_err())?;

        let mut req = client.request(parsed_method, url.clone());
        if let Some(headers) = t.headers {
            for header in headers {
                req = req.header(header.name, header.value);
            }
        }
        if let Some(body) = t.body {
            req = req.body(body);
        }

        let response = req
            .send()
            .await
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let status = response.status().as_u16();
        let content_type = Self::content_type(&response);

        let body_bytes = Self::read_body(response)
            .await
            .map_err(|e| DrunError::internal(e).into_tool_err())?;

        let is_html = content_type.to_lowercase().contains("text/html");
        let (dir, filename) = Self::bundle_paths(&url, is_html);
        let saved_to = format!("{dir}/{filename}");
        let bytes_len = body_bytes.len();

        let asset_urls = if is_html {
            Self::discover_asset_urls(&String::from_utf8_lossy(&body_bytes), &url)
        } else {
            Vec::new()
        };

        let mut used_names: HashSet<String> = [filename.clone(), "manifest.json".to_string()]
            .into_iter()
            .collect();
        let mut files = vec![(saved_to.clone(), body_bytes)];
        let mut fetched = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();

        for asset_url in &asset_urls {
            let resolved = asset_url.as_str().to_string();
            let host = asset_url
                .host_str()
                .expect("http/https urls always have a host");
            if !config.domain_allowed(host) {
                skipped.push(json!({"url": resolved, "reason": "domain_not_allowed"}));
                continue;
            }
            match Self::fetch_asset(&client, asset_url).await {
                Ok((asset_status, asset_content_type, asset_bytes)) => {
                    let asset_bytes_len = asset_bytes.len();
                    let asset_name = Self::asset_filename(asset_url, &mut used_names);
                    let asset_saved_to = format!("{dir}/{asset_name}");
                    files.push((asset_saved_to.clone(), asset_bytes));
                    fetched.push(json!({
                        "url": resolved,
                        "saved_to": asset_saved_to,
                        "bytes": asset_bytes_len,
                        "content_type": asset_content_type,
                        "status": asset_status,
                    }));
                }
                Err(reason) => {
                    failed.push(json!({"url": resolved, "reason": reason}));
                }
            }
        }

        let manifest_path = format!("{dir}/manifest.json");
        let (fetched_count, skipped_count, failed_count) =
            (fetched.len(), skipped.len(), failed.len());
        files.push((
            manifest_path.clone(),
            serde_json::to_vec_pretty(&json!({
                "source_url": url.as_str(),
                "saved_to": saved_to,
                "assets": fetched,
                "skipped": skipped,
                "failed": failed,
                "totals": {
                    "fetched": fetched_count,
                    "skipped": skipped_count,
                    "failed": failed_count,
                },
            }))
            .unwrap_or_default(),
        ));

        self.with_session_mut(&session_id, |session| {
            session
                .write_files(files, "session_fetch", None)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(ResponseBuilder::text(
                json!({
                    "status": status,
                    "bytes": bytes_len,
                    "content_type": content_type,
                    "saved_to": saved_to,
                    "dir": dir,
                    "manifest_path": manifest_path,
                    "assets_fetched": fetched_count,
                    "assets_skipped": skipped_count,
                    "assets_failed": failed_count,
                })
                .to_string(),
            ))
        })
    }

    fn bundle_paths(url: &Url, is_html: bool) -> (String, String) {
        let host = url.host_str().expect("http/https urls always have a host");
        let segment = url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .filter(|s| !s.is_empty());
        let slug = match segment.and_then(|s| s.rsplit_once('.')) {
            Some((stem, _)) if !stem.is_empty() => Some(stem),
            _ => segment,
        };
        let dir = match slug {
            Some(slug) => format!("downloads/{host}/{slug}"),
            None => format!("downloads/{host}"),
        };
        let filename = match segment {
            Some(s) => s.to_string(),
            None if is_html => "index.html".to_string(),
            None => "download".to_string(),
        };
        (dir, filename)
    }

    fn discover_asset_urls(html: &str, base: &Url) -> Vec<Url> {
        let doc = Html::parse_document(html);
        let mut urls = Vec::new();
        let mut seen = HashSet::new();
        let mut push = |raw: &str| {
            if let Some(url) = Self::resolve_asset_url(base, raw)
                && seen.insert(url.as_str().to_string())
            {
                urls.push(url);
            }
        };

        let link_sel = Selector::parse("link[href]").expect("valid selector");
        for el in doc.select(&link_sel) {
            let rel = el.value().attr("rel").unwrap_or("").to_lowercase();
            let is_asset_rel = rel
                .split_whitespace()
                .any(|r| r == "stylesheet" || r.ends_with("icon"));
            if is_asset_rel && let Some(href) = el.value().attr("href") {
                push(href);
            }
        }

        for selector in ["script[src]", "img[src]"] {
            let sel = Selector::parse(selector).expect("valid selector");
            for el in doc.select(&sel) {
                if let Some(src) = el.value().attr("src") {
                    push(src);
                }
            }
        }

        urls
    }

    fn resolve_asset_url(base: &Url, raw: &str) -> Option<Url> {
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("data:")
            || trimmed.starts_with("mailto:")
            || trimmed.starts_with("javascript:")
        {
            return None;
        }
        let resolved = base.join(trimmed).ok()?;
        matches!(resolved.scheme(), "http" | "https").then_some(resolved)
    }

    async fn fetch_asset(
        client: &reqwest::Client,
        url: &Url,
    ) -> Result<(u16, String, Vec<u8>), String> {
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("http_{}", status.as_u16()));
        }
        let content_type = Self::content_type(&response);
        let bytes = Self::read_body(response).await?;
        Ok((status.as_u16(), content_type, bytes))
    }

    fn content_type(response: &reqwest::Response) -> String {
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }

    async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    fn asset_filename(url: &Url, used: &mut HashSet<String>) -> String {
        let raw = url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .filter(|s| !s.is_empty())
            .unwrap_or("asset");
        let mut name = raw.to_string();
        if used.contains(&name) {
            let (stem, ext) = match name.rsplit_once('.') {
                Some((stem, ext)) => (stem.to_string(), Some(ext.to_string())),
                None => (name.clone(), None),
            };
            let mut i = 2;
            loop {
                let candidate = match &ext {
                    Some(ext) => format!("{stem}-{i}.{ext}"),
                    None => format!("{stem}-{i}"),
                };
                if !used.contains(&candidate) {
                    name = candidate;
                    break;
                }
                i += 1;
            }
        }
        used.insert(name.clone());
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use crate::tools::HttpHeader;
    use drun_core::Config;

    fn fetch_test_config(mock_uri: &str) -> Config {
        Config {
            domain_allowlist: vec![
                Url::parse(mock_uri)
                    .unwrap()
                    .host_str()
                    .unwrap()
                    .to_string(),
            ],
            ..Config::default()
        }
    }

    fn fetch(url: String) -> SessionFetch {
        SessionFetch {
            url,
            method: None,
            headers: None,
            body: None,
        }
    }

    #[tokio::test]
    async fn session_fetch_returns_no_active_session_without_a_current_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_fetch(CLIENT, fetch("https://pypi.org/simple/".to_string()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no_active_session"));
    }

    #[tokio::test]
    async fn session_fetch_denies_urls_outside_the_domain_allowlist() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(CLIENT, fetch("https://evil.example.com/data".to_string()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fetch_denied"));
    }

    #[tokio::test]
    async fn session_fetch_denies_an_unsupported_scheme() {
        let handler = DrunHandler::new(Config {
            domain_allowlist: vec!["example.com".to_string()],
            ..Config::default()
        });
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(CLIENT, fetch("ftp://example.com/foo".to_string()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fetch_denied"));
    }

    #[tokio::test]
    async fn session_fetch_denies_an_unparseable_url() {
        let handler = DrunHandler::new(Config::default());
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(CLIENT, fetch("not a url".to_string()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fetch_denied"));
    }

    #[tokio::test]
    async fn session_fetch_rejects_an_invalid_http_method() {
        use wiremock::MockServer;

        let mock_server = MockServer::start().await;
        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    method: Some("IN VALID".to_string()),
                    ..fetch(mock_server.uri())
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
            .handle_session_fetch(CLIENT, fetch(mock_server.uri()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[tokio::test]
    async fn session_fetch_forwards_custom_headers_and_method_to_the_request() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("x-api-key", "secret"))
            .respond_with(ResponseTemplate::new(201).set_body_string("created"))
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(
                CLIENT,
                SessionFetch {
                    method: Some("post".to_string()),
                    headers: Some(vec![HttpHeader {
                        name: "x-api-key".to_string(),
                        value: "secret".to_string(),
                    }]),
                    body: Some("payload".to_string()),
                    ..fetch(mock_server.uri())
                },
            )
            .await
            .unwrap();
        assert_eq!(result_json(&result)["status"], 201);
    }

    #[tokio::test]
    async fn session_fetch_saves_a_non_html_response_under_a_url_derived_folder() {
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
            .handle_session_fetch(CLIENT, fetch(format!("{}/data.json", mock_server.uri())))
            .await
            .unwrap();

        let json = result_json(&result);
        assert_eq!(json["status"], 200);
        assert_eq!(json["dir"], "downloads/127.0.0.1/data");
        assert_eq!(json["saved_to"], "downloads/127.0.0.1/data/data.json");
        assert_eq!(json["assets_fetched"], 0);

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        let files = &session.current().files;
        assert_eq!(
            files
                .get("downloads/127.0.0.1/data/data.json")
                .unwrap()
                .as_slice(),
            br#"{"ok":true}"#
        );
        assert!(files.contains_key("downloads/127.0.0.1/data/manifest.json"));
    }

    #[tokio::test]
    async fn session_fetch_bundles_an_html_pages_linked_assets() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let html = r#"<html><head>
            <link rel="stylesheet" href="css/style.css">
            <script src="/js/app.js"></script>
        </head><body>
            <img src="img/logo.png">
            <script src="https://evil.example.com/track.js"></script>
        </body></html>"#;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/css/style.css"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(".x{color:red}", "text/css"))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/js/app.js"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("console.log(1)", "application/javascript"),
            )
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/img/logo.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8, 1, 2, 3]))
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_current_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(CLIENT, fetch(format!("{}/", mock_server.uri())))
            .await
            .unwrap();

        let json = result_json(&result);
        assert_eq!(json["status"], 200);
        assert_eq!(json["assets_fetched"], 3);
        assert_eq!(json["assets_skipped"], 1);
        assert_eq!(json["assets_failed"], 0);
        let dir = json["dir"].as_str().unwrap().to_string();
        assert_eq!(dir, "downloads/127.0.0.1");
        assert_eq!(json["saved_to"], format!("{dir}/index.html"));
        assert_eq!(json["manifest_path"], format!("{dir}/manifest.json"));

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        let files = &session.current().files;

        assert_eq!(
            files.get(&format!("{dir}/index.html")).unwrap().as_slice(),
            html.as_bytes()
        );
        assert_eq!(
            files.get(&format!("{dir}/style.css")).unwrap().as_slice(),
            b".x{color:red}"
        );
        assert_eq!(
            files.get(&format!("{dir}/app.js")).unwrap().as_slice(),
            b"console.log(1)"
        );
        assert_eq!(
            files.get(&format!("{dir}/logo.png")).unwrap().as_slice(),
            &[0u8, 1, 2, 3]
        );

        let manifest: serde_json::Value =
            serde_json::from_slice(files.get(&format!("{dir}/manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["totals"]["fetched"], 3);
        assert_eq!(manifest["totals"]["skipped"], 1);
        assert_eq!(manifest["totals"]["failed"], 0);
        assert_eq!(manifest["skipped"][0]["reason"], "domain_not_allowed");
    }
}
