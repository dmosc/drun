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
    description = "Create a persistent sandbox session and make it the active session for \
                   this connection — subsequent session_* calls apply to it without needing \
                   a session_id. Returns the new session_id for reference (e.g. to pass to \
                   session_switch later).",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateSession {}

#[mcp_tool(
    name = "session_switch",
    description = "Change which session is active for this connection. Every session_* tool \
                   without a session_id argument (session_bash, session_read_file, etc.) then \
                   applies to this session. Use session_list to see available session ids.",
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
    description = "Run a shell command in the active session's workspace. The current \
                   checkpoint's files are materialized into a temporary directory and the \
                   command runs there via sh -c with the host PATH — so any binary installed \
                   on the host (python3, node, ruby, go, etc.) is available. Directories \
                   registered as mount_overlay_paths (node_modules, venvs, etc.) are symlinked \
                   in automatically. File changes are captured as a new checkpoint. Command \
                   policy (denylist/allowlist) is enforced by server config. Network is \
                   blocked — use session_fetch first for external data, or \
                   session_package_install to install a package.",
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
    description = "Install packages so subsequent session_bash calls can import them. Unlike \
                   session_bash, this reaches the network — but only to install into a \
                   disposable staging area, never the session's own files, so a malicious \
                   package can't use that network access to exfiltrate anything from the \
                   workspace. Installed packages persist as a new checkpoint and are \
                   automatically importable afterward (PYTHONPATH/NODE_PATH are set for you). \
                   Disabled by default; the server operator must set package_install_enabled = \
                   true. Supported package_manager values: \"pip\", \"npm\".",
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
    description = "Move the active session's head to a prior checkpoint. This is destructive: the next session_bash, session_write_file, session_delete_file, or session_merge call that succeeds permanently discards every checkpoint after the rollback point — there is no branch kept around. A call that fails (denied command, timeout, over a limit) leaves history untouched. If you want to keep the checkpoints you are rolling back past, call session_fork first (it creates a new, independent session at this point) before rolling back. Provide checkpoint_id or checkpoint_label; label takes precedence if both are given.",
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
}

#[mcp_tool(
    name = "session_read_file",
    description = "Read a file from the active session's current checkpoint by its \
                   session-relative path (e.g. src/main.py, not an absolute path). For small \
                   files or images, omit offset and limit to get the full content. For large \
                   files, use offset + limit to page through without flooding context. The \
                   response includes total_bytes and has_more so you know when you have \
                   reached the end. To locate something in a large file instead of paging \
                   blind, set pattern to a case-sensitive regex (e.g. use (?i) for \
                   case-insensitive) — offset/limit still select the byte range to search \
                   (the whole file if omitted), and the response is a list of matching lines \
                   with their line_number and byte_offset instead of raw bytes. Use the \
                   byte_offset of a match with a follow-up offset/limit read to pull \
                   surrounding context. Requires UTF-8 content.",
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
}

#[mcp_tool(
    name = "session_diff",
    description = "Compute a unified diff between two checkpoints of the active session. Defaults to comparing the initial mounted state (checkpoint 0) against the current checkpoint. Returns standard unified diff output across all changed files. Each endpoint accepts an ID or a label; label takes precedence. Pass paths to restrict the diff to specific files instead of every changed file.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionDiff {
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
    /// Path in the session to diff. Leave emtpy for all.
    pub paths: Option<Vec<String>>,
}

#[mcp_tool(
    name = "session_mount",
    description = "Copy a file or directory from the host filesystem into the active session. \
                   A file at /host/foo.py is accessible as foo.py; a directory at \
                   /host/myproject/ is accessible as myproject/. Directories whose names match \
                   mount_overlay_paths (node_modules, venvs, etc.) are registered as read-only \
                   host overlays — symlinked at execution time and never loaded into memory.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionMount {
    /// Absolute path to a file or directory on the host filesystem.
    pub path: String,
}

#[mcp_tool(
    name = "session_extract_text",
    description = "Extract plain text from a binary document already in the workspace \
                   (currently PDF only) and save it as a new file — session_bash can't read \
                   PDF bytes directly, so mount or session_fetch the file first, then call \
                   this before processing it. Runs in-process, no network or sandbox involved. \
                   Defaults save_to to path with .txt appended; read the result with \
                   session_read_file.",
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
    description = "List all active sessions with their checkpoint count and parent references. \
                   is_current marks the session active for this connection.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionList {}

#[mcp_tool(
    name = "session_close",
    description = "Terminate the active session and free all associated resources including \
                   the sandbox subprocess. Switch to a different session first if you meant to \
                   close one other than the active one.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionClose {}

#[mcp_tool(
    name = "session_history",
    description = "List every checkpoint in the active session with stdout_bytes/stderr_bytes \
                   and the file delta relative to the previous checkpoint. Use \
                   checkpoint_read_stdstreams to read the actual output. Use this to decide \
                   which checkpoint to roll back to.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionHistory {}

#[mcp_tool(
    name = "get_session_state",
    description = "Get the current state of the active session: checkpoint id, \
                   stdout_bytes/stderr_bytes, file list, and deltas since the previous \
                   checkpoint. stdout and stderr are not returned inline — use \
                   checkpoint_read_stdstreams to page through them.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSessionState {}

#[mcp_tool(
    name = "session_write_file",
    description = "Create or overwrite a file in the active session by its session-relative \
                   path (e.g. src/main.py). Creates a new checkpoint. Set is_base64 to true to \
                   write binary files — content will be decoded from standard base64 before \
                   writing.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionWriteFile {
    /// Session-relative file path (e.g. src/main.py).
    pub path: String,
    pub content: String,
    pub is_base64: Option<bool>,
    pub description: String,
}

#[mcp_tool(
    name = "session_delete_file",
    description = "Delete a file from the active session's workspace. Creates a new checkpoint.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionDeleteFile {
    /// Session-relative file path (e.g. src/main.py).
    pub path: String,
    pub description: String,
}

#[mcp_tool(
    name = "session_export",
    description = "Write the active session's workspace files to a host directory. By default \
                   writes every current file into output_dir; pass keys to select a subset. \
                   Only ever creates/overwrites — never deletes anything at output_dir, even if \
                   a file was deleted from the session after being mounted. output_dir doesn't \
                   need to be the directory session_mount was called with, or a mount at all — \
                   the session doesn't track where anything was originally mounted from. \
                   output_dir is subject to the same mount_allowlist as session_mount — check \
                   get_config to see permitted prefixes.",
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
}

#[mcp_tool(
    name = "session_merge",
    description = "Overlay files from another session's checkpoint onto the active session, \
                   creating a new checkpoint with the merged workspace. Useful for combining \
                   the best parts of two parallel explorations. Provide keys to merge only \
                   specific files; omit to merge all files from the source. Accepts \
                   checkpoint_id or checkpoint_label on the source; label takes precedence. \
                   Defaults to the source session's current checkpoint. Like session_bash and \
                   session_write_file, this discards any checkpoints ahead of the current head \
                   left by a prior session_rollback.",
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
    description = "Create a new session branching from the active session at a given \
                   checkpoint, and make the fork the active session for this connection. The \
                   fork inherits the workspace files from the source. All runtime limits \
                   (timeouts, network policy, etc.) are governed by server config and are \
                   identical across all sessions. Returns the new session_id. Provide \
                   checkpoint_id or checkpoint_label to branch from a specific point; label \
                   takes precedence. Omit both to branch from the current checkpoint.",
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
    description = "Return the full, always-current guide to using drun's tools: getting \
                   started, how to resume a session you didn't start, reading command output, \
                   and every tool grouped by purpose. Call this before your first drun tool \
                   call in a session.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetSystemInstructions {}

#[mcp_tool(
    name = "get_config",
    description = "Return the server's operator-configured limits and allowlists: which domains \
                   session_fetch may reach, which host paths session_mount may load, which env \
                   vars session_get_env may read, the bash command policy, whether \
                   session_package_install is enabled, and resource limits (workspace size, \
                   checkpoint count, timeouts). Call this before your first session_fetch, \
                   session_mount, or session_package_install to see what's available instead of \
                   discovering it through denied calls. Note the allowlists default oppositely \
                   when empty: an empty domain_allowlist permits no domains, while an empty \
                   mount_allowlist permits any host path.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetConfig {}

#[mcp_tool(
    name = "session_fetch",
    description = "The designated gateway for all outbound HTTP. session_bash has no network \
                   access by design — fetch external data here first, then process it there. \
                   Makes an HTTP request from the host and saves the response into a folder \
                   named after the URL under downloads/<host>/<page>/ in the active session, \
                   immediately available to subsequent session_bash calls. If the response is \
                   HTML, its linked stylesheets, scripts, and images are also fetched (a \
                   shallow scan, not a full render) into the same folder. That folder always \
                   has a manifest.json listing what was fetched, skipped, or failed. Response \
                   bodies are never returned inline — use session_read_file with offset + \
                   limit to read them in chunks. The target URL's domain, and every asset \
                   domain, must be in the server's fetch allowlist.",
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
}

#[mcp_tool(
    name = "session_tree",
    description = "Return the full session-checkpoint tree in a single call. Root sessions are top-level; forks are nested under the checkpoint they branched from. Each checkpoint is flagged with is_current so you can see the active head of every session at a glance, and carries the tool, command, description, and file delta counts recorded when it was created — enough to reconstruct what happened and why across every session without switching into each one. Call this first when picking up a session you did not create, or when resuming work after a break, to rebuild context before acting.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionTree {}

#[mcp_tool(
    name = "list_snapshots",
    description = "List all .drun snapshot files in the server's snapshots directory. Returns \
                   path, size, label, and checkpoint count for each file. \
                   Use session_restore to reload any entry.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListSnapshots {}

#[mcp_tool(
    name = "session_snapshot",
    description = "Serialize the active session's full checkpoint history to a .drun file on \
                   the host. Captures all checkpoints and workspace files. Returns the path \
                   the file was written to. Use session_restore to reload it.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionSnapshotTool {
    /// Absolute path for the output .drun file. Defaults to ./drun-snapshots/<session_id>.drun.
    pub path: Option<String>,
}

#[mcp_tool(
    name = "session_restore",
    description = "Load a session from a .drun snapshot file, restoring all checkpoint \
                   history and workspace files, and make it the active session for this \
                   connection. Returns the new session_id.",
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
    description = "Read a host environment variable by name. Only variables listed in the server's env_allowlist may be read. Use this to pass secrets (API keys, tokens) into the active session without hardcoding them.",
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
    description = "Attach a human-readable label to the active session. The label appears in session_list, session_state, and session_tree to make it easy to identify what a session is for. Pass an empty string to clear the label.",
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
    description = "Attach a human-readable label to a checkpoint in the active session. Labels appear in session_history and session_tree. Useful for marking milestones like 'data loaded', 'model trained', or 'baseline'. Omit checkpoint_id to label the current checkpoint. Pass an empty string to clear the label.",
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
}

#[mcp_tool(
    name = "session_checkpoint_squash",
    description = "Collapse a range of checkpoints in the active session into one, keeping the \
                   terminal file state and merging all stdout/stderr. Useful for cleaning up \
                   exploration history before committing to a direction. The range is \
                   inclusive on both ends and must start at checkpoint 1 or later — checkpoint \
                   0 is the baseline session_diff compares against by default, so it can never \
                   be folded into a squash. Returns the updated checkpoint history.",
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
}

#[mcp_tool(
    name = "checkpoint_read_stdstreams",
    description = "Read stdout or stderr from a checkpoint in the active session with offset \
                   and limit for pagination. Tool calls like session_bash, session_history, \
                   and get_session_state report stdout_bytes/stderr_bytes but do not return \
                   the content inline — use this tool to fetch it. Defaults to the current \
                   checkpoint's stdout. Returns the same offset/length/total_bytes/has_more \
                   envelope as session_read_file. To locate something in a large stream \
                   instead of paging blind, set pattern to a case-sensitive regex (e.g. use \
                   (?i) for case-insensitive) — offset/limit still select the byte range to \
                   search (the whole stream if omitted), and the response is a list of \
                   matching lines with their line_number and byte_offset instead of raw bytes. \
                   Use the byte_offset of a match with a follow-up offset/limit read to pull \
                   surrounding context.",
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
}

#[mcp_tool(
    name = "session_checkpoint_drop",
    description = "Remove a range of checkpoints from the active session's history to free \
                   memory or stay under the checkpoint limit. The range is inclusive on both \
                   ends and must start at checkpoint 1 or later — checkpoint 0 is the baseline \
                   session_diff compares against by default, so it can never be dropped. Cannot \
                   drop the current checkpoint. Returns the updated checkpoint history.",
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
        SessionWriteFile,
        SessionDeleteFile,
        SessionMount,
        SessionExtractText,
        SessionDiff,
        SessionExport,
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
    ]
);
