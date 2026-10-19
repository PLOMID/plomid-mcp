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
// Backend-neutral database contracts for bounded reads,
// controlled writes, health checks, and schema metadata operations.
use std::time::Duration;

use anyhow::Result;

#[async_trait::async_trait]
pub trait PlomidDatabase: Send + Sync {
    async fn health(&self) -> Result<DatabaseHealth>;
    async fn query(&self, request: QueryRequest) -> Result<QueryResult>;
    async fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult>;
    async fn list_databases(&self) -> Result<Vec<String>>;
    async fn list_schemas(&self) -> Result<Vec<String>>;
    async fn list_tables(&self, schema: Option<String>) -> Result<Vec<TableInfo>>;
    async fn describe_table(&self, schema: String, table: String) -> Result<TableDescription>;
}

#[derive(Clone, Debug)]
pub struct QueryRequest {
    pub sql: String,
    pub parameters: Vec<String>,
    pub row_limit: u32,
    pub timeout: Duration,
    pub max_result_bytes: usize,
    /// Schemas permitted for this authenticated request. Empty means the
    /// database role's normal search path and privileges remain authoritative.
    pub allowed_schemas: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ExecuteRequest {
    pub sql: String,
    pub parameters: Vec<String>,
    pub timeout: Duration,
    pub allowed_schemas: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ExecuteResult {
    pub affected_rows: u64,
    pub committed: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct QueryResponse {
    pub schema_version: &'static str,
    pub data: QueryResult,
    pub meta: QueryMeta,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct QueryMeta {
    pub request_id: String,
    pub elapsed_ms: u128,
    pub truncated: bool,
    pub warnings: Vec<String>,
    pub retryable: bool,
}

#[derive(Clone, Debug)]
pub struct DatabaseHealth {
    pub connected: bool,
    pub server_version: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub ordinal_position: i32,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TableDescription {
    pub schema: String,
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub keys: Vec<KeyInfo>,
    pub indexes: Vec<IndexInfo>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct KeyInfo {
    pub name: String,
    pub kind: String,
    pub columns: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct IndexInfo {
    pub name: String,
    pub definition: String,
}
