//! MCP tool schemas exposed to clients. Each struct maps to one callable tool
//! with its input parameters and hints.

use rust_mcp_sdk::{
    macros::{JsonSchema, mcp_tool},
    tool_box,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[mcp_tool(
    name = "create_session",
    description = "Create a persistent sandbox session. Returns a session_id for subsequent calls.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateSessionTool {}

#[mcp_tool(
    name = "session_execute",
    description = "Run Python code in an existing session, building on the current checkpoint. Returns stdout and the new checkpoint_id.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionExecuteTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Python source code to run
    pub code: String,
}

#[mcp_tool(
    name = "session_rollback",
    description = "Move the session head to a prior checkpoint without discarding history. Subsequent writes branch from the new head. Use session_fork if you want to explore a branch while keeping the original.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionRollbackTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Checkpoint ID to restore.
    pub checkpoint_id: u64,
}

#[mcp_tool(
    name = "session_install_package",
    description = "Install a Python package into the session. The package will be available in all subsequent session_execute calls.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionInstallPackageTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Package name as it appears on PyPI.
    pub package: String,
}

#[mcp_tool(
    name = "session_read_file",
    description = "Read a file from the current session checkpoint. \
                   For small files or images, omit offset and limit to get the full content. \
                   For large files (fetched data, CSVs, reports), use offset + limit to page \
                   through the content without flooding context. The response includes total_bytes \
                   and has_more so you know when you have reached the end.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionReadFileTool {
    /// Session ID from create_session
    pub session_id: String,
    /// File path relative to /workspace.
    pub path: String,
    /// Byte offset to start reading from. Omit to start from the beginning.
    pub offset: Option<u64>,
    /// Maximum number of bytes to return. Omit to return all remaining bytes.
    pub limit: Option<u64>,
}

#[mcp_tool(
    name = "session_diff",
    description = "Compute a unified diff between two checkpoints. Defaults to comparing the initial mounted state (checkpoint 0) against the current checkpoint. Returns standard unified diff output across all changed files.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionDiffTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Checkpoint to diff from. Defaults to 0 (the mounted state).
    pub from_checkpoint_id: Option<u64>,
    /// Checkpoint to diff to. Defaults to the current checkpoint.
    pub to_checkpoint_id: Option<u64>,
}

#[mcp_tool(
    name = "session_mount",
    description = "Copy a file or directory from the host filesystem into the session workspace. Files become available at /workspace/<filename> (or /workspace/<relative-path> for directories). Call before session_execute to make host data accessible to the sandbox.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionMountTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Absolute path to a file or directory on the host filesystem.
    pub path: String,
}

#[mcp_tool(
    name = "session_list",
    description = "List all active sessions with their checkpoint count, installed packages, and resource limits.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionListTool {}

#[mcp_tool(
    name = "session_close",
    description = "Terminate a session and free all associated resources including the sandbox subprocess.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCloseTool {
    /// Session ID from create_session.
    pub session_id: String,
}

#[mcp_tool(
    name = "session_history",
    description = "List every checkpoint in a session with its stdout and the file delta relative to the previous checkpoint. Use this to decide which checkpoint to roll back to.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionHistoryTool {
    /// Session ID from create_session.
    pub session_id: String,
}

#[mcp_tool(
    name = "get_session_state",
    description = "Get the current state of a session: workspace files, installed packages, and checkpoint info.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSessionStateTool {
    /// Session ID from create_session.
    pub session_id: String,
}

#[mcp_tool(
    name = "session_write_file",
    description = "Create or overwrite a file in the session workspace. Creates a new checkpoint. Path is relative to /workspace. Set is_base64 to true to write binary files — content will be decoded from standard base64 before writing.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionWriteFileTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// File path relative to /workspace.
    pub path: String,
    pub content: String,
    pub is_base64: Option<bool>,
}

#[mcp_tool(
    name = "session_delete_file",
    description = "Delete a file from the session workspace. Creates a new checkpoint.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionDeleteFileTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// File path relative to /workspace.
    pub path: String,
}

#[mcp_tool(
    name = "session_export",
    description = "Write sandbox-generated files to the host filesystem. By default exports all files with no host origin (i.e. created inside the sandbox, not from session_mount) into output_dir. Pass keys to select specific files. output_dir defaults to ./drun-export/<session> in the current working directory.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionExportTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Absolute path to a directory on the host to write files into. Defaults to ./drun-export/<session_id>.
    pub output_dir: Option<String>,
    /// Specific workspace-relative file keys to export. Omit to export all sandbox-generated files.
    pub keys: Option<Vec<String>>,
}

#[mcp_tool(
    name = "session_fork",
    description = "Create a new session branching from an existing session at a given checkpoint. The fork inherits the workspace files and installed packages from the source. All runtime limits (timeouts, network policy, etc.) are governed by server config and are identical across all sessions. Returns a new session_id independent of the original.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionForkTool {
    /// Session ID to fork from.
    pub session_id: String,
    /// Checkpoint to branch from. Defaults to the current checkpoint.
    pub checkpoint_id: Option<u64>,
}

#[mcp_tool(
    name = "session_commit",
    description = "Write changed files back to their original host paths. Only files that were mounted and have changed since mounting are written. Pass specific keys to commit a subset, or omit to commit all changed mounted files.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCommitTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Specific file keys to commit. Omit to commit all changed mounted files.
    pub keys: Option<Vec<String>>,
}

#[mcp_tool(
    name = "get_fetch_allowlist",
    description = "Return the list of domains the server permits for session_fetch calls and Python outbound HTTP. Use this to check what external hosts are available before constructing fetch requests.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetFetchAllowlistTool {}

#[mcp_tool(
    name = "get_allowed_packages",
    description = "Return the list of packages the server permits for session_install_package. \
                   An empty list means all packages are allowed. A non-empty list means only those \
                   packages may be installed — calls with any other package name will be rejected. \
                   Check this before attempting to install a package.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetAllowedPackagesTool {}

#[mcp_tool(
    name = "session_fetch",
    description = "Make an HTTP request from the host and save the response body as a workspace file. \
                   The body is never returned inline — it is always written to the workspace so large \
                   payloads never flood your context. Returns metadata: status, bytes, content_type, and \
                   the workspace path where the body was saved. Use session_read_file with offset + limit \
                   to read the file in chunks. The target URL's domain must be in the server's fetch allowlist.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionFetchTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Fully-qualified URL to request.
    pub url: String,
    /// HTTP method. Defaults to GET.
    pub method: Option<String>,
    /// Request headers as name/value pairs.
    pub headers: Option<Vec<HttpHeader>>,
    /// Request body for POST/PUT/PATCH.
    pub body: Option<String>,
    /// Workspace-relative path where the response body will be saved.
    pub save_to: Option<String>,
}

#[mcp_tool(
    name = "session_tree",
    description = "Return the full session-checkpoint tree in a single call. Root sessions are top-level; forks are nested under the checkpoint they branched from. Each checkpoint is flagged with is_current so you can see the active head of every session at a glance.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionTreeTool {}

#[mcp_tool(
    name = "session_snapshot",
    description = "Serialize a session's full checkpoint history to a .drun file on the host. \
                   Captures all checkpoints, installed packages, and session config. \
                   Returns the path the file was written to. Use session_restore to reload it.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionSnapshotTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Absolute path for the output .drun file. Defaults to ./drun-snapshots/<session_id>.drun.
    pub path: Option<String>,
}

#[mcp_tool(
    name = "session_restore",
    description = "Load a session from a .drun snapshot file. Reinstalls packages and restores \
                   all checkpoint history. Returns a new session_id ready for use.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionRestoreTool {
    /// Absolute path to the .drun snapshot file to load.
    pub path: String,
}

#[mcp_tool(
    name = "session_get_env",
    description = "Read a host environment variable by name. Only variables listed in the server's env_allowlist may be read. Use this to pass secrets (API keys, tokens) into a session without hardcoding them.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionGetEnvTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Name of the environment variable to read.
    pub name: String,
}

#[mcp_tool(
    name = "session_label",
    description = "Attach a human-readable label to a session. The label appears in session_list, session_state, and session_tree to make it easy to identify what a session is for. Pass an empty string to clear the label.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionLabelTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Human-readable label for the session. Empty string clears the label.
    pub label: String,
}

#[mcp_tool(
    name = "session_checkpoint_label",
    description = "Attach a human-readable label to a checkpoint. Labels appear in session_history and session_tree. Useful for marking milestones like 'data loaded', 'model trained', or 'baseline'. Omit checkpoint_id to label the current checkpoint. Pass an empty string to clear the label.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCheckpointLabelTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Checkpoint to label. Defaults to the current checkpoint.
    pub checkpoint_id: Option<u64>,
    /// Human-readable label for the checkpoint. Empty string clears the label.
    pub label: String,
}

tool_box!(
    DrunTools,
    [
        CreateSessionTool,
        SessionForkTool,
        SessionListTool,
        SessionCloseTool,
        SessionHistoryTool,
        GetSessionStateTool,
        SessionInstallPackageTool,
        SessionExecuteTool,
        SessionRollbackTool,
        SessionReadFileTool,
        SessionWriteFileTool,
        SessionDeleteFileTool,
        SessionMountTool,
        SessionDiffTool,
        SessionCommitTool,
        SessionExportTool,
        SessionTreeTool,
        SessionFetchTool,
        GetFetchAllowlistTool,
        GetAllowedPackagesTool,
        SessionSnapshotTool,
        SessionRestoreTool,
        SessionGetEnvTool,
        SessionLabelTool,
        SessionCheckpointLabelTool,
    ]
);
