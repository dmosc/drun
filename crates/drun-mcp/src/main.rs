mod bridge;
mod cli;
mod config_cmd;
mod drunmon;
mod env;
mod errors;
mod file_manager;
mod handler;
mod live_output;
mod response;
mod server;
mod state;
mod tools;
mod web;

pub(crate) use cli::Cli;
pub(crate) use config_cmd::ConfigCmd;
pub(crate) use env::Env;
pub(crate) use file_manager::FileManager;
use handler::DrunHandler;
pub(crate) use response::ResponseBuilder;
use rust_mcp_sdk::{
    ToMcpServerHandler,
    error::SdkResult,
    mcp_server::{HyperServerOptions, hyper_server},
};

#[tokio::main]
async fn main() -> SdkResult<()> {
    match std::env::args().nth(1).as_deref() {
        Some("bridges") => {
            match std::env::args().nth(2).as_deref() {
                Some("list") => bridge::REGISTRY.print_list(),
                Some("deregister-all") => bridge::REGISTRY.deregister_all(),
                _ => {
                    Cli::print_usage();
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some("config") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            ConfigCmd::run(&rest);
            return Ok(());
        }
        Some("--help" | "-h") => {
            Cli::print_usage();
            return Ok(());
        }
        Some(name) => {
            let Some(bridge) = bridge::REGISTRY.find(name) else {
                Cli::print_usage();
                std::process::exit(1);
            };
            match std::env::args().nth(2).as_deref() {
                Some("init") => bridge.init(),
                Some("deregister") => bridge.deregister(),
                _ => {
                    Cli::print_usage();
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        None => {}
    }

    let started_at = std::time::Instant::now();
    let handler = DrunHandler::new_live();
    handler.start_idle_reaper();
    handler.start_shutdown_handler();
    handler.start_drunmon_push();

    if let Some(web_port) = handler.config.get().web_port.filter(|&p| p != 0) {
        tokio::spawn(web::WebServer::new(handler.clone(), web_port, started_at).serve());
    }

    let mcp_port = Env.mcp_port();
    eprintln!("drun: MCP listening at http://127.0.0.1:{mcp_port}/mcp (streamable HTTP)");
    eprintln!("drun: MCP listening at http://127.0.0.1:{mcp_port}/sse (SSE)");

    hyper_server::create_server(
        Cli::build_server_details(),
        handler.to_mcp_server_handler(),
        HyperServerOptions {
            host: "127.0.0.1".into(),
            port: mcp_port,
            ..Default::default()
        },
    )
    .start()
    .await
}
