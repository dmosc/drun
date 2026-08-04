use crate::ResponseBuilder;
use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::state::SessionState;
use crate::tools::SessionPackageInstall;
use rust_mcp_sdk::{
    McpServer,
    schema::{CallToolResult, ProgressToken, schema_utils::CallToolError},
};
use std::sync::Arc;

impl DrunHandler {
    pub(super) async fn handle_session_package_install(
        &self,
        connection_id: &str,
        t: SessionPackageInstall,
        runtime: Arc<dyn McpServer>,
        progress_token: Option<ProgressToken>,
    ) -> Result<CallToolResult, CallToolError> {
        let session_id = self.current_sessions.resolve(connection_id)?;
        let progress_tx = Self::spawn_progress_forwarder(runtime, progress_token);
        let handler = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            handler
                .with_session_mut(&session_id, |session| {
                    let label = format!("{} install {}", t.package_manager, t.packages.join(" "));
                    let live_output = handler.live_output.start(&session_id, &label);
                    let previous_files = session.current().files.clone();
                    session
                        .install_package(
                            &t.package_manager,
                            &t.packages,
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
}
