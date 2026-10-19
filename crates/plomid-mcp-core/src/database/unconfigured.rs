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
// Safe unavailable-backend implementation used when no PLOMID
// connection is configured, preserving deterministic MCP error behavior.
use super::{
    DatabaseHealth, ExecuteRequest, ExecuteResult, PlomidDatabase, QueryRequest, QueryResult,
    TableDescription, TableInfo,
};
use anyhow::Result;

#[derive(Debug, Default)]
pub struct UnconfiguredDatabase;
impl UnconfiguredDatabase {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl PlomidDatabase for UnconfiguredDatabase {
    async fn health(&self) -> Result<DatabaseHealth> {
        Ok(DatabaseHealth {
            connected: false,
            server_version: None,
        })
    }
    async fn query(&self, _request: QueryRequest) -> Result<QueryResult> {
        anyhow::bail!("PLOMID database adapter is not connected")
    }
    async fn execute(&self, _request: ExecuteRequest) -> Result<ExecuteResult> {
        anyhow::bail!("PLOMID database adapter is not connected")
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        anyhow::bail!("PLOMID database adapter is not connected")
    }
    async fn list_schemas(&self) -> Result<Vec<String>> {
        anyhow::bail!("PLOMID database adapter is not connected")
    }
    async fn list_tables(&self, _schema: Option<String>) -> Result<Vec<TableInfo>> {
        anyhow::bail!("PLOMID database adapter is not connected")
    }
    async fn describe_table(&self, _schema: String, _table: String) -> Result<TableDescription> {
        anyhow::bail!("PLOMID database adapter is not connected")
    }
}
