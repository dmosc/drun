mod bridges;
mod config_cmd;
mod errors;
mod handler;
mod live_output;
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

pub(crate) const DEFAULT_MCP_PORT: u16 = 7273;

pub(crate) fn mcp_port() -> u16 {
    std::env::var("DRUN_MCP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_MCP_PORT)
}

/// `~/.drun` — shared across `config_cmd` and any `Bridge` that needs it
/// (e.g. `bridges::claude` for the project registry and mount allowlist).
/// Not bridge-specific: lives here rather than in any one bridge module.
pub(crate) fn drun_home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").expect("HOME not set")).join(".drun")
}

#[tokio::main]
async fn main() -> SdkResult<()> {
    match std::env::args().nth(1).as_deref() {
        Some("bridges") => {
            match std::env::args().nth(2).as_deref() {
                Some("list") => bridges::print_list(),
                Some("deregister-all") => bridges::deregister_all(),
                _ => {
                    print_usage();
                    std::process::exit(1);
                }
            }
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
        Some(name) => {
            let Some(bridge) = bridges::find(name) else {
                print_usage();
                std::process::exit(1);
            };
            match std::env::args().nth(2).as_deref() {
                Some("init") => bridge.init(),
                Some("deregister") => bridge.deregister(),
                _ => {
                    print_usage();
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

    if let Some(web_port) = handler.config.get().web_port.filter(|&p| p != 0) {
        tokio::spawn(web::WebServer::new(handler.clone(), web_port, started_at).serve());
    }

    let mcp_port = mcp_port();
    eprintln!("drun: MCP → http://127.0.0.1:{mcp_port}/mcp (streamable HTTP)");
    eprintln!("drun: MCP → http://127.0.0.1:{mcp_port}/sse (SSE)");

    hyper_server::create_server(
        build_server_details(),
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

fn print_usage() {
    let mut rows: Vec<(String, String)> = vec![
        (String::new(), "start the daemon".into()),
        ("bridges list".into(), "list available agent bridges".into()),
        (
            "bridges deregister-all".into(),
            "undo every registered bridge".into(),
        ),
    ];
    // One `init`/`deregister` pair per registered bridge — adding a bridge
    // to `bridges::REGISTRY` makes it show up here automatically.
    for bridge in bridges::REGISTRY {
        rows.push((
            format!("{} init", bridge.name()),
            bridge.description().to_string(),
        ));
        rows.push((
            format!("{} deregister", bridge.name()),
            format!("undo `{} init`", bridge.name()),
        ));
    }
    rows.extend(
        [
            (
                "config add-domain <name>",
                "allow a domain for session_fetch",
            ),
            ("config add-path <path>", "allow a path for session_mount"),
            (
                "config remove-domain <name>",
                "disallow a domain for session_fetch",
            ),
            (
                "config remove-path <path>",
                "disallow a path for session_mount",
            ),
            ("config list", "show the current allowlists"),
        ]
        .map(|(cmd, desc)| (cmd.to_string(), desc.to_string())),
    );

    let width = rows.iter().map(|(cmd, _)| cmd.len()).max().unwrap_or(0);

    let mut usage = String::from("drun-mcp — MCP server for drun\n\nUsage:\n");
    for (cmd, desc) in &rows {
        usage.push_str(&format!("  drun-mcp {cmd:<width$}  {desc}\n"));
    }
    usage.push_str(
        "\nconfig add-*/remove-* edit ~/.drun/config.toml — changes take effect\n\
         on the next tool call, no restart needed.",
    );

    eprintln!("{usage}");
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
