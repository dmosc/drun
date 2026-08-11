//! MCP tool schemas exposed to clients. Each struct maps to one callable tool
//! with its input parameters and hints. Session-scoped tools act on the
//! connection's active session (see session_switch) rather than taking a
//! session_id.

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
    description = "Create a sandbox session and make it active for this connection — later \
                   session_* calls need no session_id. Returns the new session_id.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateSession {}

#[mcp_tool(
    name = "session_switch",
    description = "Change which session is active for this connection; session_* calls without \
                   session_id then apply to it. See session_list for available ids.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionSwitch {
    /// Session ID to make active, from create_session, session_fork, session_restore, or session_list.
    pub session_id: String,
}

#[mcp_tool(
    name = "session_bash",
    description = "Run a shell command in the active session's workspace via sh -c. Host PATH \
                   tools (python3, node, etc.) are available; dirs matching mount_overlay_paths \
                   (node_modules, venvs) are symlinked in automatically. File changes become a \
                   new checkpoint. No network — use session_fetch first, or \
                   session_package_install for packages. Subject to server command policy.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionBash {
    /// Shell command to run (passed to sh -c).
    pub command: String,
    pub description: String,
}

#[mcp_tool(
    name = "session_package_install",
    description = "Install pip/npm packages so session_bash can import them. Network access is \
                   limited to a disposable staging area, never the session's own files. \
                   Persists as a new checkpoint; PYTHONPATH/NODE_PATH are set automatically. \
                   Disabled by default — the operator must set package_install_enabled = true.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionPackageInstall {
    /// Package manager to install with: "pip" or "npm".
    pub package_manager: String,
    /// Package specifiers to install (e.g. "requests", "left-pad@1.3.0").
    pub packages: Vec<String>,
    pub description: String,
}

#[mcp_tool(
    name = "session_rollback",
    description = "Move the active session's head to a prior checkpoint. Destructive: the next \
                   successful mutating call permanently discards everything after the rollback \
                   point — no branch is kept. Call session_fork first if you want to keep that \
                   history. checkpoint_label takes precedence over checkpoint_id if both are given.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionRollback {
    /// Checkpoint ID to restore. Provide this or checkpoint_label.
    pub checkpoint_id: Option<u64>,
    /// Label of the checkpoint to restore. Takes precedence over checkpoint_id.
    pub checkpoint_label: Option<String>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_read_file",
    description = "Read one file by session-relative path. Omit offset/limit for the full \
                   content (or an image). For large files, page with offset/limit, or set \
                   pattern (case-sensitive regex; use (?i) for case-insensitive) to search \
                   instead of paging blind — returns matching lines with line_number and \
                   byte_offset. Use a match's byte_offset in a follow-up offset/limit read for \
                   context. To read several files at once, use session_read_files instead.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionReadFile {
    /// Session-relative file path (e.g. src/main.py).
    pub path: String,
    /// Byte offset to start reading from. Omit to start from the beginning.
    pub offset: Option<u64>,
    /// Maximum number of bytes to return. Omit to return all remaining bytes.
    pub limit: Option<u64>,
    /// Case-sensitive regex; when set, returns matching lines within the
    /// offset/limit byte range instead of raw content.
    pub pattern: Option<String>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_read_files",
    description = "Read the full content of one or more files by session-relative path, in one \
                   call — batch instead of one call per file. Returns each file's content and \
                   encoding (text or base64) in the given order. For a single large file needing \
                   offset/limit paging or pattern search, use session_read_file instead.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionReadFiles {
    /// Session-relative file paths to read.
    pub paths: Vec<String>,
    pub description: String,
}

#[mcp_tool(
    name = "session_diff",
    description = "Unified diff between two checkpoints, defaulting to current vs. the previous \
                   one. Pass from_checkpoint_id to go further back. A *_label field takes \
                   precedence over its *_id counterpart. Pass paths to restrict to specific files.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionDiff {
    /// Checkpoint to diff from. Defaults to the previous one.
    pub from_checkpoint_id: Option<u64>,
    /// Label of the checkpoint to diff from. Takes precedence over
    /// from_checkpoint_id.
    pub from_checkpoint_label: Option<String>,
    /// Checkpoint to diff to. Defaults to the current checkpoint.
    pub to_checkpoint_id: Option<u64>,
    /// Label of the checkpoint to diff to. Takes precedence over
    /// to_checkpoint_id.
    pub to_checkpoint_label: Option<String>,
    /// Path in the session to diff. Leave emtpy for all.
    pub paths: Option<Vec<String>>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_mount",
    description = "Copy a host file or directory into the active session. A file at \
                   /host/foo.py becomes foo.py; a directory becomes its own name. Dirs matching \
                   mount_overlay_paths (node_modules, venvs) are symlinked read-only instead of \
                   copied.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionMount {
    /// Absolute path to a file or directory on the host filesystem.
    pub path: String,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_extract_text",
    description = "Extract plain text from a binary document in the workspace (PDF only) and \
                   save it as a new file — mount or session_fetch the file first. Defaults \
                   save_to to path + \".txt\"; read the result with session_read_file.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionExtractText {
    /// Session-relative path to the source document (e.g. report.pdf).
    pub path: String,
    /// Session-relative path to save the extracted text to. Defaults to path + ".txt".
    pub save_to: Option<String>,
    pub description: String,
}

#[mcp_tool(
    name = "session_list",
    description = "List all active sessions with checkpoint count and parent reference. \
                   is_current marks this connection's active session.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionList {}

#[mcp_tool(
    name = "session_close",
    description = "Terminate the active session and free its resources, including the sandbox \
                   subprocess. Switch sessions first if closing one other than the active one.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionClose {}

#[mcp_tool(
    name = "session_history",
    description = "List every checkpoint with stdout_bytes/stderr_bytes and the file delta vs. \
                   the previous one. Use checkpoint_read_stdstreams for the actual output.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionHistory {
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "get_session_state",
    description = "Current session state: checkpoint id, stdout_bytes/stderr_bytes, file list, \
                   and deltas since the previous checkpoint. Use checkpoint_read_stdstreams for \
                   stdout/stderr content.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSessionState {
    /// Description documenting the operation.
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FileWrite {
    /// Session-relative file path (e.g. src/main.py).
    pub path: String,
    pub content: String,
    pub is_base64: Option<bool>,
}

#[mcp_tool(
    name = "session_write_files",
    description = "Create or overwrite one or more files by session-relative path, as a single \
                   checkpoint — batch instead of one call per file. is_base64 on an entry \
                   decodes its content from base64 before writing.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionWriteFiles {
    pub entries: Vec<FileWrite>,
    pub description: String,
}

#[mcp_tool(
    name = "session_delete_files",
    description = "Delete one or more files from the workspace as a single checkpoint — batch \
                   instead of one call per path.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionDeleteFiles {
    /// Session-relative file paths to delete.
    pub paths: Vec<String>,
    pub description: String,
}

#[mcp_tool(
    name = "session_export",
    description = "Write workspace files to a host directory. Defaults to every file; pass keys \
                   for a subset. Only ever creates/overwrites, never deletes. Subject to the \
                   mount_allowlist — see get_config.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionExport {
    /// Absolute path to a directory on the host to write files into. Must be under one of the
    /// server's mount_allowlist prefixes.
    pub output_dir: String,
    /// Specific workspace-relative file keys to export. Omit to export every current file.
    pub keys: Option<Vec<String>>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "delete_from_host",
    description = "Delete a file or directory on the host filesystem. No-ops if already gone. \
                   The only way to make a host deletion happen — session_export only \
                   creates/overwrites. Subject to the mount_allowlist — see get_config.",
    idempotent_hint = true,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DeleteFromHost {
    /// Absolute path to a file or directory on the host filesystem to delete.
    /// Must be under one of the server's mount_allowlist prefixes.
    pub path: String,
}

#[mcp_tool(
    name = "session_merge",
    description = "Overlay files from another session's checkpoint onto the active session as a \
                   new checkpoint. keys restricts which files merge; omit for all. A *_label \
                   field takes precedence over its *_id counterpart; defaults to the source's \
                   current checkpoint. Like session_bash, discards any checkpoints left ahead of \
                   the head by a prior session_rollback.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionMerge {
    /// Session ID to merge files from (the source). The target is the active session.
    pub source_session_id: String,
    /// Checkpoint on the source to merge from. Defaults to the source's current checkpoint.
    pub source_checkpoint_id: Option<u64>,
    /// Label of the checkpoint on the source to merge from. Takes precedence over
    /// source_checkpoint_id.
    pub source_checkpoint_label: Option<String>,
    /// Specific file paths to merge. Omit to merge all files from the source checkpoint.
    pub keys: Option<Vec<String>>,
    pub description: String,
}

#[mcp_tool(
    name = "session_fork",
    description = "Branch a new session from a checkpoint of the active one, and make it active. \
                   Inherits the source's workspace files; runtime limits are server-wide and \
                   identical across sessions. Returns the new session_id. checkpoint_label takes \
                   precedence over checkpoint_id; omit both to branch from the current checkpoint.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionFork {
    /// Checkpoint to branch from. Defaults to the current checkpoint.
    pub checkpoint_id: Option<u64>,
    /// Label of the checkpoint to branch from. Takes precedence over
    /// checkpoint_id.
    pub checkpoint_label: Option<String>,
}

#[mcp_tool(
    name = "get_system_instructions",
    description = "Full, always-current guide to drun's tools: getting started, resuming a \
                   session you didn't start, reading command output, and every tool by purpose. \
                   Call before your first drun tool call in a session.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSystemInstructions {}

#[mcp_tool(
    name = "get_config",
    description = "Server-configured limits and allowlists: fetch domains, mount paths, readable \
                   env vars, bash command policy, whether session_package_install is enabled, \
                   and resource limits. Call before session_fetch/session_mount/ \
                   session_package_install to see what's available instead of hitting denials. \
                   Empty domain_allowlist permits no domains; empty mount_allowlist permits any \
                   host path — the defaults are opposite.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetConfig {}

#[mcp_tool(
    name = "session_fetch",
    description = "The gateway for outbound HTTP — session_bash has no network access. Saves the \
                   response (and, for HTML, its linked assets) under \
                   downloads/<host>/<page>/ in the session, with a manifest.json of what was \
                   fetched. Body is never returned inline — read it with session_read_file. The \
                   target domain, and every asset domain, must be in the server's fetch allowlist.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionFetch {
    /// Fully-qualified URL to request.
    pub url: String,
    /// HTTP method. Defaults to GET.
    pub method: Option<String>,
    /// Request headers as name/value pairs.
    pub headers: Option<Vec<HttpHeader>>,
    /// Request body for POST/PUT/PATCH.
    pub body: Option<String>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_tree",
    description = "Full session-checkpoint tree in one call: forks nested under their branch \
                   point, is_current flagging each active head, each checkpoint carrying the \
                   tool/command/description/file-delta counts recorded when it was made. Call \
                   first when picking up a session you didn't create, to rebuild context.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionTree {}

#[mcp_tool(
    name = "list_snapshots",
    description = "List .drun snapshot files in the server's snapshots directory: path, size, \
                   label, checkpoint count. Use session_restore to reload one.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListSnapshots {}

#[mcp_tool(
    name = "session_snapshot",
    description = "Serialize the active session's full checkpoint history and workspace files to \
                   a .drun file on the host. Returns the output path. Use session_restore to \
                   reload it.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionSnapshotTool {
    /// Absolute path for the output .drun file. Defaults to
    /// ~/.drun/snapshots/<session_id>.drun.
    pub path: Option<String>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_restore",
    description = "Load a session from a .drun snapshot file, restoring its full checkpoint \
                   history and files, and make it active. Returns the new session_id.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionRestore {
    /// Absolute path to the .drun snapshot file to load.
    pub path: String,
}

#[mcp_tool(
    name = "session_get_env",
    description = "Read a host environment variable by name (must be in the server's \
                   env_allowlist). Use this to pass secrets into the session without \
                   hardcoding them.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionGetEnv {
    /// Name of the environment variable to read.
    pub name: String,
}

#[mcp_tool(
    name = "session_label",
    description = "Attach a human-readable label to the active session, shown in session_list, \
                   get_session_state, and session_tree. Empty string clears it.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionLabel {
    /// Human-readable label for the session. Empty string clears the label.
    pub label: String,
}

#[mcp_tool(
    name = "session_checkpoint_label",
    description = "Attach a human-readable label to a checkpoint, shown in session_history and \
                   session_tree. Omit checkpoint_id to label the current checkpoint. Empty \
                   string clears the label.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCheckpointLabel {
    /// Checkpoint to label. Defaults to the current checkpoint.
    pub checkpoint_id: Option<u64>,
    /// Human-readable label for the checkpoint. Empty string clears the label.
    pub label: String,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_checkpoint_squash",
    description = "Collapse a checkpoint range into one, keeping the terminal file state and \
                   merging all stdout/stderr. Range is inclusive and must start at checkpoint 1 \
                   or later (0 is the empty starting point, never squashable). Returns the \
                   updated checkpoint history.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCheckpointSquash {
    /// First checkpoint in the range to squash (inclusive).
    pub from_checkpoint_id: u64,
    /// Last checkpoint in the range to squash (inclusive).
    pub to_checkpoint_id: u64,
    /// Optional label to attach to the resulting squashed checkpoint.
    pub label: Option<String>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "checkpoint_read_stdstreams",
    description = "Read stdout or stderr from a checkpoint, paginated with offset/limit. Tools \
                   like session_bash report byte counts but not content — use this to fetch it. \
                   Defaults to the current checkpoint's stdout. Set pattern (case-sensitive \
                   regex; use (?i) for case-insensitive) to search within the range instead of \
                   paging blind — returns matching lines with line_number and byte_offset.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CheckpointReadStdstreams {
    /// Checkpoint to read output from. Defaults to the current checkpoint.
    pub checkpoint_id: Option<u64>,
    /// Stream to read: "stdout" (default) or "stderr".
    pub stream: Option<String>,
    /// Byte offset to start reading from. Omit to start from the beginning.
    pub offset: Option<u64>,
    /// Maximum number of bytes to return. Omit to return all remaining bytes.
    pub limit: Option<u64>,
    /// Case-sensitive regex; when set, returns matching lines within the
    /// offset/limit byte range instead of raw content.
    pub pattern: Option<String>,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_checkpoint_drop",
    description = "Remove a checkpoint range from history to save memory or stay under the \
                   limit. Range is inclusive and must start at checkpoint 1 or later; the \
                   current checkpoint can't be dropped. Returns the updated checkpoint history.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionCheckpointDrop {
    /// First checkpoint in the range to drop (inclusive).
    pub from_checkpoint_id: u64,
    /// Last checkpoint in the range to drop (inclusive).
    pub to_checkpoint_id: u64,
    /// Description documenting the operation.
    pub description: String,
}

#[mcp_tool(
    name = "session_chat_record",
    description = "Record a prompt/response exchange in the session's chat log, so it \
                   round-trips through session_snapshot/session_restore. The drun chat CLI calls \
                   this automatically — never call it directly.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionChatRecord {
    /// The user's prompt for this exchange.
    pub prompt: String,
    /// The agent's final response to the prompt.
    pub response: String,
}

tool_box!(
    DrunTools,
    [
        CreateSession,
        SessionSwitch,
        SessionFork,
        SessionList,
        SessionClose,
        SessionHistory,
        GetSessionState,
        SessionBash,
        SessionPackageInstall,
        SessionRollback,
        SessionReadFile,
        SessionReadFiles,
        SessionWriteFiles,
        SessionDeleteFiles,
        SessionMount,
        SessionExtractText,
        SessionDiff,
        SessionExport,
        DeleteFromHost,
        SessionTree,
        SessionFetch,
        GetConfig,
        GetSystemInstructions,
        ListSnapshots,
        SessionSnapshotTool,
        SessionRestore,
        SessionGetEnv,
        SessionLabel,
        SessionCheckpointLabel,
        SessionCheckpointSquash,
        SessionCheckpointDrop,
        SessionMerge,
        CheckpointReadStdstreams,
        SessionChatRecord,
    ]
);
