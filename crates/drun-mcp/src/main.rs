//! MCP server entry point. Wires up the transport, server metadata, and
//! handler, then starts the stdio loop.

mod errors;
mod handler;
mod reaper;
mod response;
mod server;
mod state;
mod tools;

use drun_core::Config;
use handler::DrunHandler;
use rust_mcp_sdk::{
    McpServer, StdioTransport, ToMcpServerHandler, TransportOptions,
    error::SdkResult,
    mcp_server::{McpServerOptions, server_runtime},
    schema::{
        Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
        ServerCapabilitiesTools,
    },
};

#[tokio::main]
async fn main() -> SdkResult<()> {
    let handler = DrunHandler::new(Config::load());
    handler.start_idle_reaper();
    let handler = handler.to_mcp_server_handler();

    let server = server_runtime::create_server(McpServerOptions {
        server_details: InitializeResult {
            server_info: Implementation {
                name: "drun".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("drun".into()),
                description: Some("Sandboxed code execution for agentic loops".into()),
                icons: vec![],
                website_url: None,
            },
            capabilities: ServerCapabilities {
                tools: Some(ServerCapabilitiesTools { list_changed: None }),
                ..Default::default()
            },
            protocol_version: ProtocolVersion::V2025_11_25.into(),
            instructions: Some("Go to https://drun.dev to view docs.".into()),
            meta: None,
        },
        transport: StdioTransport::new(TransportOptions::default())?,
        handler,
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    server.start().await
}
