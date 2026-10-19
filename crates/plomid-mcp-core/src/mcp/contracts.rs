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
// Stable JSON schemas for status, metadata requests, and
// controlled-write responses exposed through the MCP contract.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StatusResponse {
    pub schema_version: &'static str,
    pub ok: bool,
    pub mode: &'static str,
    pub database_configured: bool,
    pub database_status: &'static str,
    pub warnings: Vec<&'static str>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SchemaRequest {
    #[serde(default)]
    #[schemars(schema_with = "nullable_string_schema")]
    pub schema: Option<String>,
}

fn nullable_string_schema(generator: &mut SchemaGenerator) -> Schema {
    let string_schema = String::json_schema(generator);
    json_schema!({
        "anyOf": [string_schema, {"type": "null"}]
    })
}

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DescribeTableRequest {
    pub schema: String,
    pub table: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ExecuteResponse {
    pub schema_version: &'static str,
    pub data: crate::database::ExecuteResult,
    pub committed: bool,
}
