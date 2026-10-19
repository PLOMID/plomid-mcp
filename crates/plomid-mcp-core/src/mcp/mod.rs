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
// MCP server composition, backend lifecycle, write idempotency,
// and protocol-level instructions for safe PLOMID operations.

mod contracts;
mod tools;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, PromptMessage, Role, ServerCapabilities, ServerConfig},
    prompt, prompt_handler, prompt_router, tool_handler,
};

use crate::{
    config::{Config, SharedConfig},
    database::{DatabaseHealth, PlomidDatabase, PlomidSqlDatabase, UnconfiguredDatabase},
};

pub use contracts::StatusResponse;

const INSTRUCTIONS: &str = "Workflow: first call plomid.capabilities and inspect feature_domains, then use plomid.sql.query for bounded read-only queries and plomid.sql.explain for query plans. Read-only is the default safety mode; never attempt mutations unless an explicitly authorized mutation tool is advertised.";

#[derive(Clone)]
pub struct McpServer {
    pub(crate) config: SharedConfig,
    pub(crate) database: Arc<dyn PlomidDatabase>,
    pub(crate) write_idempotency: Arc<tokio::sync::Mutex<HashMap<String, Instant>>>,
    tool_router: ToolRouter<Self>,
    prompt_router: rmcp::handler::server::router::prompt::PromptRouter<Self>,
}

impl McpServer {
    pub fn new(config: Config) -> Result<Self> {
        let database: Arc<dyn PlomidDatabase> = match config.database_url.as_ref() {
            Some(url) => Arc::new(PlomidSqlDatabase::new(
                url,
                config.pool_size,
                config.max_concurrent_queries,
            )?),
            None => Arc::new(UnconfiguredDatabase::new()),
        };
        Self::with_database(config, database)
    }

    pub fn with_database(config: Config, database: Arc<dyn PlomidDatabase>) -> Result<Self> {
        let mut tool_router = Self::tool_router();
        if config.mode != crate::config::OperatingMode::ControlledWrite || !config.write_enabled {
            tool_router.disable_route("plomid.sql.execute");
        }
        Ok(Self {
            config: Arc::new(config),
            database,
            write_idempotency: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            tool_router,
            prompt_router: Self::prompt_router(),
        })
    }

    pub async fn database_health(&self) -> Result<DatabaseHealth> {
        self.database.health().await
    }

    pub(crate) async fn claim_write(&self, key: &str, ttl: Duration) -> bool {
        let now = Instant::now();
        let mut entries = self.write_idempotency.lock().await;
        entries.retain(|_, expiry| *expiry > now);
        if entries.contains_key(key) {
            return false;
        }
        entries.insert(key.to_owned(), now + ttl);
        true
    }
}

#[prompt_handler(router = self.prompt_router)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("plomid-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }
}

#[prompt_router]
impl McpServer {
    /// Describe the safe, read-only PLOMID query workflow.
    #[prompt(
        name = "plomid.safe-query-workflow",
        description = "Guide an MCP client through safe PLOMID schema discovery and bounded read-only querying."
    )]
    async fn safe_query_workflow(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Use plomid.capabilities first. Discover only authorized schemas and tables, bind all values as query parameters, keep SQL to one read-only statement, and honor the server's row, byte, and execution-time limits. Never infer that a timed-out mutation did not commit. Do not attempt writes unless an explicitly authorized write tool is advertised.",
        )]
    }
}
