use crate::config::Config;
use crate::response::{err, file_content, text};
use crate::state::{
    build_checkpoint_history, build_session_list, build_session_state, build_session_tree,
};
use crate::tools::DrunTools;
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use drun_core::{DrunEngine, PYTHON_PACKAGE_HOSTS, Session};
use rust_mcp_sdk::{
    McpServer,
    mcp_server::ServerHandler,
    schema::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
        schema_utils::CallToolError,
    },
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub struct DrunHandler {
    engine: DrunEngine,
    sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
    fetch_allowlist: Vec<String>,
}

impl DrunHandler {
    pub fn new(config: Config) -> Self {
        Self {
            engine: DrunEngine::new(config.session.max_workspace_mb.map(|mb| mb * 1024 * 1024))
                .expect("failed to initialize drun engine"),
            sessions: Mutex::new(HashMap::new()),
            fetch_allowlist: config.fetch.allowlist,
        }
    }

    fn build_allowed_hosts(&self, requested: Option<Vec<String>>) -> Vec<String> {
        if let Some(hosts) = requested {
            return hosts;
        }
        if self.fetch_allowlist.iter().any(|h| h == "*") {
            return vec!["*".to_string()];
        }
        let mut hosts: Vec<String> = PYTHON_PACKAGE_HOSTS.iter().map(|s| s.to_string()).collect();
        for host in &self.fetch_allowlist {
            if !hosts.contains(host) {
                hosts.push(host.clone());
            }
        }
        hosts
    }

    fn with_session(
        &self,
        session_id: &str,
        f: impl FnOnce(&Session) -> Result<CallToolResult, CallToolError>,
    ) -> Result<CallToolResult, CallToolError> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .ok_or_else(|| err(format!("session '{}' not found", session_id)))?
            .clone();
        f(&session.lock().unwrap())
    }

    fn with_session_mut(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut Session) -> Result<CallToolResult, CallToolError>,
    ) -> Result<CallToolResult, CallToolError> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .ok_or_else(|| err(format!("session '{}' not found", session_id)))?
            .clone();
        f(&mut session.lock().unwrap())
    }
}

#[async_trait]
impl ServerHandler for DrunHandler {
    async fn handle_list_tools_request(
        &self,
        _params: Option<PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, RpcError> {
        Ok(ListToolsResult {
            tools: DrunTools::tools(),
            meta: None,
            next_cursor: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let tool = DrunTools::try_from(params)?;
        match tool {
            DrunTools::CreateSessionTool(t) => {
                let session_id = Uuid::new_v4().to_string();
                let allowed_hosts = self.build_allowed_hosts(t.allowed_hosts);
                let session =
                    Session::new(&self.engine, allowed_hosts, t.timeout_ms).map_err(err)?;
                let state = build_session_state(&session_id, &session, None, vec![]);
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(session_id, Arc::new(Mutex::new(session)));
                Ok(text(state))
            }

            DrunTools::SessionForkTool(t) => {
                let source_session = {
                    let sessions = self.sessions.lock().unwrap();
                    sessions
                        .get(&t.session_id)
                        .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?
                        .clone()
                };
                let forked_session = {
                    let source = source_session.lock().unwrap();
                    Session::from_session(
                        &self.engine,
                        &t.session_id,
                        &source,
                        t.checkpoint_id.map(|id| id as usize),
                    )
                    .map_err(err)?
                };
                let fork_session_id = Uuid::new_v4().to_string();
                let session_state =
                    build_session_state(&fork_session_id, &forked_session, None, vec![]);
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(fork_session_id, Arc::new(Mutex::new(forked_session)));
                Ok(text(session_state))
            }

            DrunTools::SessionListTool(_) => {
                let sessions = self.sessions.lock().unwrap().clone();
                Ok(text(build_session_list(&sessions)))
            }

            DrunTools::SessionCloseTool(t) => {
                let removed_session = self.sessions.lock().unwrap().remove(&t.session_id);
                if removed_session.is_none() {
                    return Err(err(format!("session '{}' not found", t.session_id)));
                }
                Ok(text(format!("closed {}", t.session_id)))
            }

            DrunTools::SessionHistoryTool(t) => self.with_session(&t.session_id, |session| {
                Ok(text(build_checkpoint_history(session)))
            }),

            DrunTools::GetSessionStateTool(t) => self.with_session(&t.session_id, |session| {
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    None,
                    vec![],
                )))
            }),

            DrunTools::SessionInstallPackageTool(t) => {
                self.with_session_mut(&t.session_id, |session| {
                    session.install(&t.package).map_err(err)?;
                    Ok(text(build_session_state(
                        &t.session_id,
                        session,
                        None,
                        vec![],
                    )))
                })
            }

            DrunTools::SessionExecuteTool(t) => self.with_session_mut(&t.session_id, |session| {
                let previous_files = session.current().files.clone();
                session.execute(&t.code).map_err(err)?;
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }),

            DrunTools::SessionRollbackTool(t) => self.with_session_mut(&t.session_id, |session| {
                let previous_files = session.current().files.clone();
                session.rollback(t.checkpoint_id as usize).map_err(err)?;
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }),

            DrunTools::SessionReadFileTool(t) => self.with_session(&t.session_id, |session| {
                let bytes = session
                    .current()
                    .files
                    .get(&t.path)
                    .ok_or_else(|| err(format!("'{}' not in current checkpoint", t.path)))?;
                Ok(file_content(&t.path, bytes))
            }),

            DrunTools::SessionWriteFileTool(t) => self.with_session_mut(&t.session_id, |session| {
                let bytes = if t.is_base64.unwrap_or(false) {
                    BASE64
                        .decode(&t.content)
                        .map_err(|e| err(format!("base64 decode error: {e}")))?
                } else {
                    t.content.into_bytes()
                };
                let previous_files = session.current().files.clone();
                session.write_file(&t.path, bytes).map_err(err)?;
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }),

            DrunTools::SessionDeleteFileTool(t) => {
                self.with_session_mut(&t.session_id, |session| {
                    let previous_files = session.current().files.clone();
                    session.delete_file(&t.path).map_err(err)?;
                    Ok(text(build_session_state(
                        &t.session_id,
                        session,
                        Some(&previous_files),
                        vec![],
                    )))
                })
            }

            DrunTools::SessionMountTool(t) => self.with_session_mut(&t.session_id, |session| {
                let previous_files = session.current().files.clone();
                session.mount(std::path::Path::new(&t.path)).map_err(err)?;
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }),

            DrunTools::SessionDiffTool(t) => self.with_session(&t.session_id, |session| {
                let from = t.from_checkpoint_id.unwrap_or(0) as usize;
                let to = t
                    .to_checkpoint_id
                    .map(|id| id as usize)
                    .unwrap_or_else(|| session.current().id);
                let diff = session.diff(from, to).map_err(err)?;
                Ok(text(if diff.is_empty() {
                    "no changes".into()
                } else {
                    diff
                }))
            }),

            DrunTools::SessionCommitTool(t) => self.with_session(&t.session_id, |session| {
                let paths = session.commit(t.keys).map_err(err)?;
                let committed_files = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    None,
                    committed_files,
                )))
            }),

            DrunTools::SessionTreeTool(_) => {
                let sessions: HashMap<String, Arc<Mutex<Session>>> =
                    self.sessions.lock().unwrap().clone();
                Ok(text(build_session_tree(&sessions)))
            }

            DrunTools::SessionExportTool(t) => {
                static DEFAULT_EXPORT_FOLDER: &str = "drun-export";
                self.with_session(&t.session_id, |session| {
                    let output_dir = match &t.output_dir {
                        Some(dir) => std::path::PathBuf::from(dir),
                        None => std::env::current_dir()
                            .map_err(err)?
                            .join(DEFAULT_EXPORT_FOLDER)
                            .join(&t.session_id),
                    };
                    let exported_files = session.export(&output_dir, t.keys).map_err(err)?;
                    let exported_paths: Vec<String> = exported_files
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    Ok(text(
                        serde_json::json!({
                            "output_dir": output_dir.to_string_lossy(),
                            "exported_files": exported_paths,
                        })
                        .to_string(),
                    ))
                })
            }

            DrunTools::SessionFetchTool(t) => {
                if !self.sessions.lock().unwrap().contains_key(&t.session_id) {
                    return Err(err(format!("session '{}' not found", t.session_id)));
                }
                let url_is_allowed = self.fetch_allowlist.iter().any(|h| h == "*")
                    || host_from_url(&t.url).map_or(false, |h| self.fetch_allowlist.contains(&h));
                if !url_is_allowed {
                    return Err(err(format!(
                        "'{}' is not permitted by the server's fetch allowlist",
                        t.url
                    )));
                }

                let method = t.method.as_deref().unwrap_or("GET").to_uppercase();
                let parsed_method = method
                    .parse::<reqwest::Method>()
                    .map_err(|_| err(format!("invalid HTTP method: {}", method)))?;

                let client = reqwest::Client::new();
                let mut request_builder = client.request(parsed_method, &t.url);
                if let Some(headers) = t.headers {
                    for header in headers {
                        request_builder = request_builder.header(header.name, header.value);
                    }
                }
                if let Some(body) = t.body {
                    request_builder = request_builder.body(body);
                }

                let response = request_builder
                    .send()
                    .await
                    .map_err(|e| err(e.to_string()))?;
                let status = response.status().as_u16();
                let response_headers: HashMap<String, String> = response
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                    .collect();
                let body_bytes = response.bytes().await.map_err(|e| err(e.to_string()))?;
                let (body, body_encoding) = match std::str::from_utf8(&body_bytes) {
                    Ok(text) => (text.to_string(), None),
                    Err(_) => (BASE64.encode(&body_bytes), Some("base64")),
                };

                Ok(text(
                    serde_json::to_string(&FetchResponse {
                        status,
                        headers: response_headers,
                        body,
                        body_encoding,
                    })
                    .unwrap(),
                ))
            }

            DrunTools::GetFetchAllowlistTool(_) => {
                Ok(text(serde_json::to_string(&self.fetch_allowlist).unwrap()))
            }
        }
    }
}

fn host_from_url(url: &str) -> Option<String> {
    let s = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = s.split('/').next().filter(|h| !h.is_empty())?;
    let host = if authority.starts_with('[') {
        // IPv6 literal — do not attempt to strip port
        authority.to_string()
    } else {
        // Strip port if present (domain:port or IPv4:port)
        authority.split(':').next()?.to_string()
    };
    Some(host)
}

#[derive(Serialize)]
struct FetchResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_encoding: Option<&'static str>,
}
