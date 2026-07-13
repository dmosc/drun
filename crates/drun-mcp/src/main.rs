mod config_cmd;
mod errors;
mod handler;
mod init;
mod reaper;
mod response;
mod server;
mod state;
mod tools;
mod web;

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

pub(crate) const MCP_PORT: u16 = 7273;

#[tokio::main]
async fn main() -> SdkResult<()> {
    match std::env::args().nth(1).as_deref() {
        Some("init") => {
            init::run();
            return Ok(());
        }
        Some("config") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            config_cmd::run(&rest);
            return Ok(());
        }
        Some("--help" | "-h") => {
            print_usage();
            return Ok(());
        }
        _ => {}
    }

    let started_at = std::time::Instant::now();
    let handler = DrunHandler::new_live();
    handler.start_idle_reaper();
    handler.start_shutdown_handler();

    if let Some(web_port) = handler.config.get().web_port.filter(|&p| p != 0) {
        let web_sessions = std::sync::Arc::clone(&handler.sessions);
        let web_config = handler.config.clone();
        tokio::spawn(web::WebServer::new(web_sessions, web_port, web_config, started_at).serve());
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

fn print_usage() {
    eprintln!(
        "drun-mcp — MCP server for drun\n\
         \n\
         Usage:\n\
         \x20\x20drun-mcp                            start the daemon\n\
         \x20\x20drun-mcp init                        set up drun for the current project\n\
         \x20\x20drun-mcp config add-domain <name>    allow a domain for session_fetch\n\
         \x20\x20drun-mcp config add-path <path>      allow a path for session_mount\n\
         \x20\x20drun-mcp config remove-domain <name> disallow a domain for session_fetch\n\
         \x20\x20drun-mcp config remove-path <path>   disallow a path for session_mount\n\
         \x20\x20drun-mcp config list                 show the current allowlists\n\
         \n\
         config add-*/remove-* edit ~/.drun/config.toml — changes take effect\n\
         on the next tool call, no restart needed."
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_server_details_reports_the_crate_version() {
        let details = build_server_details();
        assert_eq!(details.server_info.name, "drun");
        assert_eq!(details.server_info.version, env!("CARGO_PKG_VERSION"));
    }
}
