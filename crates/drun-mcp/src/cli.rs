use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ServerCapabilitiesTools,
};

pub(crate) struct Cli;

impl Cli {
    pub(crate) fn print_usage() {
        let mut rows: Vec<(String, String)> = vec![
            (String::new(), "start the daemon".into()),
            ("bridges list".into(), "list available agent bridges".into()),
            (
                "bridges deregister-all".into(),
                "undo every registered bridge".into(),
            ),
        ];
        for bridge in crate::bridge::REGISTRY.iter() {
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

    pub(crate) fn build_server_details() -> InitializeResult {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_server_details_reports_the_crate_version() {
        let details = Cli::build_server_details();
        assert_eq!(details.server_info.name, "drun");
        assert_eq!(details.server_info.version, env!("CARGO_PKG_VERSION"));
    }
}
