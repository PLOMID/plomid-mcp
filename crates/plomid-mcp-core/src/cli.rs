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
// Command-line, environment, and configuration-file interface
// for local STDIO and secure enterprise Streamable HTTP deployments.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "plomid-mcp", version, about = "PLOMID MCP server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub settings: SettingsArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the MCP server over STDIO.
    Stdio,
    /// Run the MCP server over Streamable HTTP.
    Http,
    /// Validate configuration without starting the server.
    Validate,
    /// Print the statically known server capabilities.
    Capabilities,
    /// Check configuration and live PLOMID connectivity.
    Doctor,
}

#[derive(Debug, Args)]
pub struct SettingsArgs {
    /// Optional TOML configuration file. Environment and CLI settings override its values.
    #[arg(long, env = "PLOMID_MCP_CONFIG")]
    pub config: Option<std::path::PathBuf>,

    /// Optional PostgreSQL-wire connection URL. It is never returned or logged.
    #[arg(long, env = "PLOMID_DATABASE_URL")]
    pub database_url: Option<String>,

    /// Operating mode. Controlled writes require explicit policy, authorization, and approval.
    #[arg(long, env = "PLOMID_MCP_MODE")]
    pub mode: Option<String>,

    /// Log filter. Logs are emitted to STDERR for STDIO compatibility.
    #[arg(long, env = "RUST_LOG")]
    pub log_level: Option<String>,

    /// HTTP bind address. HTTP is opt-in and defaults to loopback.
    #[arg(long, env = "PLOMID_MCP_BIND")]
    pub bind: Option<String>,

    /// Comma-separated allowed HTTP Host values.
    #[arg(long, env = "PLOMID_MCP_ALLOWED_HOSTS", value_delimiter = ',')]
    pub allowed_hosts: Option<Vec<String>>,

    /// Comma-separated browser origins. Loopback HTTP origins are allowed by default.
    #[arg(long, env = "PLOMID_MCP_ALLOWED_ORIGINS", value_delimiter = ',')]
    pub allowed_origins: Option<Vec<String>>,

    /// Maximum HTTP request body size in bytes.
    #[arg(long, env = "PLOMID_MCP_MAX_BODY_BYTES")]
    pub max_request_body_bytes: Option<usize>,

    /// Maximum rows returned by one SQL operation.
    #[arg(long, env = "PLOMID_MCP_MAX_ROWS")]
    pub max_rows: Option<u32>,

    /// Maximum encoded result size in bytes.
    #[arg(long, env = "PLOMID_MCP_MAX_RESULT_BYTES")]
    pub max_result_bytes: Option<usize>,

    /// PostgreSQL pool size.
    #[arg(long, env = "PLOMID_MCP_POOL_SIZE")]
    pub pool_size: Option<usize>,

    /// Maximum concurrent SQL operations.
    #[arg(long, env = "PLOMID_MCP_MAX_CONCURRENT_QUERIES")]
    pub max_concurrent_queries: Option<usize>,

    /// Maximum concurrent HTTP requests.
    #[arg(long, env = "PLOMID_MCP_HTTP_CONCURRENCY")]
    pub http_concurrency: Option<usize>,

    /// Maximum HTTP requests accepted per minute per process.
    #[arg(long, env = "PLOMID_MCP_RATE_LIMIT_PER_MINUTE")]
    pub rate_limit_per_minute: Option<u32>,

    /// Enable explicitly authorized controlled writes.
    #[arg(long, env = "PLOMID_MCP_WRITE_ENABLED")]
    pub write_enabled: Option<bool>,

    /// SHA-256 digest of the private write approval token.
    #[arg(long, env = "PLOMID_MCP_WRITE_APPROVAL_TOKEN_SHA256")]
    pub write_approval_token_sha256: Option<String>,

    /// Lifetime of an approval token in seconds.
    #[arg(long, env = "PLOMID_MCP_WRITE_APPROVAL_TTL_SECS")]
    pub write_approval_ttl_secs: Option<u64>,

    /// PEM certificate path for direct HTTPS.
    #[arg(long, env = "PLOMID_MCP_TLS_CERT")]
    pub tls_cert: Option<std::path::PathBuf>,

    /// PEM private key path for direct HTTPS.
    #[arg(long, env = "PLOMID_MCP_TLS_KEY")]
    pub tls_key: Option<std::path::PathBuf>,
}
