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
    name = "session_bash",
    description = "Run a shell command in the session workspace. The current checkpoint's files \
                   are materialized into a temporary directory and the command runs there via \
                   sh -c with the host PATH — so any binary installed on the host (python3, node, \
                   ruby, go, etc.) is available. Directories registered as mount_overlay_paths \
                   (node_modules, venvs, etc.) are symlinked in automatically. File changes are \
                   captured as a new checkpoint. Command policy (denylist/allowlist) is enforced \
                   by server config. Network is blocked — use session_fetch first for external data.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionBashTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Shell command to run (passed to sh -c).
    pub command: String,
}

#[mcp_tool(
    name = "session_rollback",
    description = "Move the session head to a prior checkpoint without discarding history. Subsequent writes branch from the new head. Use session_fork if you want to explore a branch while keeping the original. Provide checkpoint_id or checkpoint_label; label takes precedence if both are given.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionRollbackTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Checkpoint ID to restore. Provide this or checkpoint_label.
    pub checkpoint_id: Option<u64>,
    /// Label of the checkpoint to restore. Takes precedence over checkpoint_id.
    pub checkpoint_label: Option<String>,
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
    description = "Compute a unified diff between two checkpoints. Defaults to comparing the initial mounted state (checkpoint 0) against the current checkpoint. Returns standard unified diff output across all changed files. Each endpoint accepts an ID or a label; label takes precedence.",
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
    /// Label of the checkpoint to diff from. Takes precedence over
    /// from_checkpoint_id.
    pub from_checkpoint_label: Option<String>,
    /// Checkpoint to diff to. Defaults to the current checkpoint.
    pub to_checkpoint_id: Option<u64>,
    /// Label of the checkpoint to diff to. Takes precedence over
    /// to_checkpoint_id.
    pub to_checkpoint_label: Option<String>,
}

#[mcp_tool(
    name = "session_mount",
    description = "Copy a file or directory from the host filesystem into the session workspace. Files become available at /workspace/<filename> (or /workspace/<relative-path> for directories). Directories whose names match mount_overlay_paths (node_modules, venvs, etc.) are registered as read-only host overlays — symlinked at execution time and never loaded into memory.",
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
    name = "session_merge",
    description = "Overlay files from another session's checkpoint onto the current session, \
                   creating a new checkpoint with the merged workspace. Useful for combining \
                   the best parts of two parallel explorations. Provide keys to merge only \
                   specific files; omit to merge all files from the source. Accepts \
                   checkpoint_id or checkpoint_label on the source; label takes precedence. \
                   Defaults to the source session's current checkpoint.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionMergeTool {
    /// Session ID to merge into (the target).
    pub session_id: String,
    /// Session ID to merge files from (the source).
    pub source_session_id: String,
    /// Checkpoint on the source to merge from. Defaults to the source's current checkpoint.
    pub source_checkpoint_id: Option<u64>,
    /// Label of the checkpoint on the source to merge from. Takes precedence over
    /// source_checkpoint_id.
    pub source_checkpoint_label: Option<String>,
    /// Specific file paths to merge. Omit to merge all files from the source checkpoint.
    pub keys: Option<Vec<String>>,
}

#[mcp_tool(
    name = "session_fork",
    description = "Create a new session branching from an existing session at a given checkpoint. The fork inherits the workspace files and installed packages from the source. All runtime limits (timeouts, network policy, etc.) are governed by server config and are identical across all sessions. Returns a new session_id independent of the original. Provide checkpoint_id or checkpoint_label to branch from a specific point; label takes precedence. Omit both to branch from the current checkpoint.",
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
    /// Label of the checkpoint to branch from. Takes precedence over
    /// checkpoint_id.
    pub checkpoint_label: Option<String>,
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
    name = "session_fetch",
    description = "The designated gateway for all outbound HTTP. session_bash has no network \
                   access by design — fetch external data here first, then process it there. \
                   Makes an HTTP request from the host and saves the response body as a workspace \
                   file so it is immediately available to subsequent session_bash calls. The body \
                   is never returned inline — use session_read_file with offset + limit to read it \
                   in chunks. The target URL's domain must be in the server's fetch allowlist.",
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
    name = "list_snapshots",
    description = "List all .drun snapshot files in the server's snapshots directory. Returns \
                   path, size, label, checkpoint count, and installed packages for each file. \
                   Use session_restore to reload any entry.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListSnapshotsTool {}

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

#[mcp_tool(
    name = "session_checkpoint_squash",
    description = "Collapse a range of checkpoints into one, keeping the terminal file state and \
                   merging all stdout/stderr. Useful for cleaning up exploration history before \
                   committing to a direction. The range is inclusive on both ends. Returns the \
                   updated checkpoint history.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCheckpointSquashTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// First checkpoint in the range to squash (inclusive).
    pub from_checkpoint_id: u64,
    /// Last checkpoint in the range to squash (inclusive).
    pub to_checkpoint_id: u64,
    /// Optional label to attach to the resulting squashed checkpoint.
    pub label: Option<String>,
}

#[mcp_tool(
    name = "session_checkpoint_drop",
    description = "Remove a range of checkpoints from history to free memory or stay under the \
                   checkpoint limit. The range is inclusive on both ends. Cannot drop the current \
                   checkpoint. Returns the updated checkpoint history.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCheckpointDropTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// First checkpoint in the range to drop (inclusive).
    pub from_checkpoint_id: u64,
    /// Last checkpoint in the range to drop (inclusive).
    pub to_checkpoint_id: u64,
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
        SessionBashTool,
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
        ListSnapshotsTool,
        SessionSnapshotTool,
        SessionRestoreTool,
        SessionGetEnvTool,
        SessionLabelTool,
        SessionCheckpointLabelTool,
        SessionCheckpointSquashTool,
        SessionCheckpointDropTool,
        SessionMergeTool,
    ]
);
