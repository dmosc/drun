//! MCP tool dispatch: routes each tool call to the `DrunHandler` method that
//! handles it — grouped one file per domain in the submodules below — and
//! wraps the result as an MCP `CallToolResult`.

mod checkpoint_ops;
mod config_snapshot;
mod execution;
mod fetch;
mod file_ops;
mod package_install;
mod session_ops;
#[cfg(test)]
mod test_support;

use crate::handler::DrunHandler;
use crate::tools::DrunTools;
use async_trait::async_trait;
use rust_mcp_sdk::{
    McpServer,
    mcp_server::ServerHandler,
    schema::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
        schema_utils::CallToolError,
    },
};
use std::sync::Arc;

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
        runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let progress_token = params.meta.as_ref().and_then(|m| m.progress_token.clone());
        let connection_id = runtime.session_id().unwrap_or_default();
        let tool = DrunTools::try_from(params)?;
        let tool_name = tool.tool_name();
        let duration = std::time::Instant::now();
        let result = match tool {
            DrunTools::CreateSession(_) => self.handle_create_session(&connection_id),
            DrunTools::SessionSwitch(t) => self.handle_session_switch(&connection_id, t),
            DrunTools::SessionFork(t) => self.handle_session_fork(&connection_id, t),
            DrunTools::SessionList(_) => self.handle_session_list(&connection_id),
            DrunTools::SessionClose(_) => self.handle_session_close(&connection_id),
            DrunTools::SessionHistory(_) => self.handle_session_history(&connection_id),
            DrunTools::GetSessionState(_) => self.handle_get_session_state(&connection_id),
            DrunTools::SessionBash(t) => {
                self.handle_session_bash(&connection_id, t, runtime, progress_token)
                    .await
            }
            DrunTools::SessionPackageInstall(t) => {
                self.handle_session_package_install(&connection_id, t, runtime, progress_token)
                    .await
            }
            DrunTools::SessionRollback(t) => self.handle_session_rollback(&connection_id, t),
            DrunTools::SessionReadFile(t) => self.handle_session_read_file(&connection_id, t),
            DrunTools::SessionWriteFile(t) => self.handle_session_write_file(&connection_id, t),
            DrunTools::SessionDeleteFile(t) => self.handle_session_delete_file(&connection_id, t),
            DrunTools::SessionMount(t) => self.handle_session_mount(&connection_id, t),
            DrunTools::SessionExtractText(t) => self.handle_session_extract_text(&connection_id, t),
            DrunTools::SessionDiff(t) => self.handle_session_diff(&connection_id, t),
            DrunTools::SessionCommit(t) => self.handle_session_commit(&connection_id, t),
            DrunTools::SessionTree(_) => self.handle_session_tree(),
            DrunTools::ListSnapshots(_) => self.handle_list_snapshots(),
            DrunTools::SessionExport(t) => self.handle_session_export(&connection_id, t),
            DrunTools::SessionFetch(t) => self.handle_session_fetch(&connection_id, t).await,
            DrunTools::GetConfig(_) => self.handle_get_config(),
            DrunTools::GetSystemInstructions(_) => self.handle_get_system_instructions(),
            DrunTools::SessionSnapshotTool(t) => self.handle_session_snapshot(&connection_id, t),
            DrunTools::SessionGetEnv(t) => self.handle_session_get_env(&connection_id, t),
            DrunTools::SessionRestore(t) => self.handle_session_restore(&connection_id, t),
            DrunTools::SessionLabel(t) => self.handle_session_label(&connection_id, t),
            DrunTools::SessionCheckpointLabel(t) => {
                self.handle_session_checkpoint_label(&connection_id, t)
            }
            DrunTools::SessionCheckpointSquash(t) => {
                self.handle_session_checkpoint_squash(&connection_id, t)
            }
            DrunTools::SessionMerge(t) => self.handle_session_merge(&connection_id, t),
            DrunTools::SessionCheckpointDrop(t) => {
                self.handle_session_checkpoint_drop(&connection_id, t)
            }
            DrunTools::CheckpointReadStdstreams(t) => {
                self.handle_checkpoint_read_stdstreams(&connection_id, t)
            }
        };
        self.tool_metrics
            .record(&tool_name, duration.elapsed(), result.is_ok());
        result
    }
}
