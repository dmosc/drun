use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::state::SessionState;
use crate::tools::SessionBash;
use rust_mcp_sdk::{
    McpServer,
    schema::{
        CallToolResult, ProgressNotificationParams, ProgressToken, schema_utils::CallToolError,
    },
};
use std::sync::Arc;

impl DrunHandler {
    pub(super) async fn handle_session_bash(
        &self,
        connection_id: &str,
        t: SessionBash,
        runtime: Arc<dyn McpServer>,
        progress_token: Option<ProgressToken>,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        let progress_tx = Self::spawn_progress_forwarder(runtime, progress_token);
        let handler = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            handler
                .with_session_mut(&session_id, |session| {
                    let live_output = handler.live_output.start(&session_id, &t.command);
                    let previous_files = session.current().files.clone();
                    session
                        .execute_bash(
                            &t.command,
                            &mut |chunk| {
                                live_output.append(&chunk);
                                let _ = progress_tx.send(chunk);
                            },
                            Some(&t.description),
                        )
                        .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
                    Ok(ResponseBuilder::json(&SessionState::compute(
                        &session_id,
                        session,
                        Some(&previous_files),
                    )))
                })
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|join_err| Err(join_err.to_string()));
        result.map_err(|msg| CallToolError(msg.into()))
    }

    pub(super) fn spawn_progress_forwarder(
        mcp_server: Arc<dyn McpServer>,
        progress_token: Option<ProgressToken>,
    ) -> tokio::sync::mpsc::UnboundedSender<String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        if let Some(token) = progress_token {
            tokio::spawn(async move {
                while let Some(chunk) = rx.recv().await {
                    let _ = mcp_server
                        .notify_progress(ProgressNotificationParams {
                            progress: 0.0,
                            progress_token: token.clone(),
                            message: Some(chunk),
                            total: None,
                            meta: None,
                        })
                        .await;
                }
            });
        }
        tx
    }
}
