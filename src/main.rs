//
// © 2026 PLOMID Technologies INDIA
//
// PLOMID — Platform for Modern Intelligence and Data
//
// Author: Sainath Sapa
// Email: info.djsai@gmail.com
// GitHub: https://github.com/sainathsapa
// Organization: https://github.com/plomid
//
// Licensed under the Apache License, Version 2.0.
//
// PLOMID-MCP executable entrypoint and lifecycle dispatch for
// STDIO, HTTPS Streamable HTTP, validation, capabilities, and diagnostics.

use anyhow::Result;
use clap::Parser;
use plomid_mcp::{
    cli::{Cli, Command},
    config::Config,
    mcp::McpServer,
};
use rmcp::ServiceExt;

const MAX_STDIO_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_cli(&cli)?;
    config.init_logging()?;

    match cli.command {
        Command::Stdio => {
            let server = McpServer::new(config)?;
            let input = plomid_mcp::transport::limited_stdio::LimitedLineReader::new(
                tokio::io::stdin(),
                MAX_STDIO_MESSAGE_BYTES,
            );
            plomid_mcp::startup::log_stdio_ready();
            let service = server.serve((input, tokio::io::stdout())).await?;
            service.waiting().await?;
        }
        Command::Http => {
            let server = McpServer::new(config.clone())?;
            plomid_mcp::transport::http::serve(server, &config).await?;
        }
        Command::Validate => {
            config.validate()?;
            eprintln!("configuration is valid");
        }
        Command::Capabilities => {
            println!("{}", serde_json::to_string_pretty(&config.capabilities())?);
        }
        Command::Doctor => {
            let server = McpServer::new(config.clone())?;
            let health = server.database_health().await?;
            println!(
                "{}",
                serde_json::json!({
                    "configuration": "valid",
                    "database_status": if health.connected { "connected" } else { "unavailable" },
                    "server_version": health.server_version,
                })
            );
        }
    }

    Ok(())
}
