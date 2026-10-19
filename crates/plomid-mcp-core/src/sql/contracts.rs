//
// © 2026 PLOMID Technologies INDIA
//
// PLOMID — Platform for Modern Intelligence and Data
//
// Author: Sainath Sapa
// Email: info.djsai@gmail.com
// GitHub: https://github.com/plomid
// Organization: https://github.com/plomid
//
// Licensed under the Apache License, Version 2.0.
//
// Validated request schemas for read, explain, and explicitly approved
// controlled-write SQL operations.
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_ROW_LIMIT: u32 = 100;
pub const MAX_ROW_LIMIT: u32 = 10_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
pub const MAX_TIMEOUT_MS: u64 = 120_000;
pub const MAX_SQL_BYTES: usize = 256 * 1024;
pub const MAX_PARAMETERS: usize = 64;
pub const MAX_PARAMETER_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SqlRequest {
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<String>,
    #[serde(default = "default_row_limit")]
    pub row_limit: u32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ExplainRequest {
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<String>,
    #[serde(default = "default_row_limit")]
    pub row_limit: u32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct WriteRequest {
    pub sql: String,
    #[serde(default)]
    pub parameters: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub approval_token: String,
    pub idempotency_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatementClass {
    Read,
    Explain,
    SessionRead,
    Mutation,
    Transaction,
    Administrative,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValidatedSql {
    pub sql: String,
    pub parameters: Vec<String>,
    pub row_limit: u32,
    pub timeout: Duration,
    pub statement_class: StatementClass,
}

#[derive(Clone, Debug)]
pub struct ValidatedWrite {
    pub sql: String,
    pub parameters: Vec<String>,
    pub timeout: Duration,
    pub idempotency_key: String,
    pub approval_token: String,
}
fn default_row_limit() -> u32 {
    DEFAULT_ROW_LIMIT
}
fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}
