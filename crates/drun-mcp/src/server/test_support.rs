//! Shared test scaffolding for the `server` submodules: a handler with an
//! inserted/current session, and result-unwrapping helpers.

use crate::handler::DrunHandler;
use drun_core::Session;
use rust_mcp_sdk::schema::{CallToolResult, ContentBlock};
use std::sync::{Arc, Mutex};

pub(crate) const CLIENT: &str = "client";

pub(crate) fn insert_session(handler: &DrunHandler, id: &str) {
    handler.sessions.lock().unwrap().insert(
        id.to_string(),
        Arc::new(Mutex::new(Session::new(handler.config.clone()).unwrap())),
    );
}

pub(crate) fn set_current(handler: &DrunHandler, id: &str) {
    handler.current_sessions.set(CLIENT, id.to_string());
}

pub(crate) fn insert_current_session(handler: &DrunHandler, id: &str) {
    insert_session(handler, id);
    set_current(handler, id);
}

pub(crate) fn result_text(result: &CallToolResult) -> &str {
    match &result.content[0] {
        ContentBlock::TextContent(tc) => &tc.text,
        _ => panic!("expected text content"),
    }
}

pub(crate) fn result_json(result: &CallToolResult) -> serde_json::Value {
    serde_json::from_str(result_text(result)).unwrap()
}
