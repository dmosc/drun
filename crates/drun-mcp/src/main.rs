mod errors;
mod handler;
mod reaper;
mod response;
mod server;
mod state;
mod tools;
mod web;

use drun_core::Config;
use handler::DrunHandler;
use rust_mcp_sdk::{
    ToMcpServerHandler,
    error::SdkResult,
    mcp_server::{HyperServerOptions, hyper_server},
    schema::{
        Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
        ServerCapabilitiesTools,
    },
};

const MCP_PORT: u16 = 7273;

#[tokio::main]
async fn main() -> SdkResult<()> {
    let handler = DrunHandler::new(Config::load());
    handler.start_idle_reaper();

    if let Some(web_port) = handler.config.web_port {
        let web_sessions = std::sync::Arc::clone(&handler.sessions);
        tokio::spawn(web::WebServer::new(web_sessions, web_port).serve());
    }

    eprintln!("drun: MCP → http://127.0.0.1:{MCP_PORT}/mcp (streamable HTTP)");
    eprintln!("drun: MCP → http://127.0.0.1:{MCP_PORT}/sse (SSE)");

    hyper_server::create_server(
        build_server_details(),
        handler.to_mcp_server_handler(),
        HyperServerOptions {
            host: "127.0.0.1".into(),
            port: MCP_PORT,
            ..Default::default()
        },
    )
    .start()
    .await
}

fn build_server_details() -> InitializeResult {
    InitializeResult {
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
    }
}
