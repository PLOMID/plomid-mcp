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
// MCP tool surface for capability discovery, bounded SQL,
// schema metadata, controlled writes, authorization, and structured results.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};

use crate::{
    config::OperatingMode,
    database::{ExecuteRequest, QueryMeta, QueryRequest, QueryResponse},
    sql::{
        ExplainRequest, SqlRequest, WriteRequest, validate_explain, validate_read_only,
        validate_schema_scope, validate_write,
    },
};
use plomid_security::{AuthPolicy, current_identity};

use super::contracts::{DescribeTableRequest, ExecuteResponse, SchemaRequest};
use super::{McpServer, StatusResponse};

fn structured<T: serde::Serialize>(value: T) -> Result<CallToolResult, rmcp::ErrorData> {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))
}

fn invalid(error: impl std::fmt::Display) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(error.to_string(), None)
}

fn backend(_error: impl std::fmt::Display) -> rmcp::ErrorData {
    // Backend messages can contain SQL identifiers, server details, or data
    // supplied by an untrusted endpoint. Keep the client error stable and
    // non-sensitive; detailed diagnostics belong in an explicitly protected
    // database-side logging system.
    tracing::warn!(
        error_class = "backend_operation_failed",
        "database operation failed"
    );
    rmcp::ErrorData::internal_error("BACKEND_UNAVAILABLE: database operation failed", None)
}

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[tool_router(vis = "pub")]
impl McpServer {
    #[tool(
        name = "plomid.status",
        description = "Report server readiness, configured mode, and database integration status without exposing secrets."
    )]
    async fn status(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("mcp:read").map_err(invalid)?;
        let mode = match self.config.mode {
            OperatingMode::ReadOnly => "read-only",
            OperatingMode::ControlledWrite => "controlled-write",
        };
        let database_health = self.database.health().await.ok();
        let database_status = match (
            self.config.database_url.is_some(),
            database_health.as_ref().map(|health| health.connected),
        ) {
            (false, _) => "not-configured",
            (true, Some(true)) => "connected",
            (true, _) => "configured-but-not-connected",
        };
        let mut warnings = Vec::new();
        if database_status != "connected" {
            warnings.push("database health check failed; retry plomid.status for current state");
        }
        structured(StatusResponse {
            schema_version: "1",
            ok: true,
            mode,
            database_configured: self.config.database_url.is_some(),
            database_status,
            warnings,
        })
    }

    #[tool(
        name = "plomid.capabilities",
        description = "Describe verified PLOMID-MCP capabilities, workload domains, resource limits, and operating mode."
    )]
    async fn capabilities(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("mcp:read").map_err(invalid)?;
        structured(self.config.capabilities())
    }

    #[tool(
        name = "plomid.sql.query",
        description = "Execute one bounded, parameterized, read-only PLOMID SQL statement across the documented relational, JSON, vector, time-series, function, aggregate, window, catalog, and EXPLAIN surface."
    )]
    async fn sql_query(
        &self,
        Parameters(request): Parameters<SqlRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("sql:read").map_err(invalid)?;
        let validated = validate_read_only(request).map_err(invalid)?;
        let allowed_schemas = current_identity()
            .map(|identity| identity.schemas)
            .unwrap_or_default();
        validate_schema_scope(&validated.sql, &allowed_schemas).map_err(invalid)?;
        self.execute_query(QueryRequest {
            sql: validated.sql,
            parameters: validated.parameters,
            row_limit: validated.row_limit.min(self.config.max_rows),
            timeout: validated.timeout,
            max_result_bytes: self.config.max_result_bytes,
            allowed_schemas,
        })
        .await
    }

    #[tool(
        name = "plomid.sql.explain",
        description = "Inspect a documented PLOMID SQL query plan using EXPLAIN, including text or JSON format and vector/aggregation plan details where the connected backend supports them."
    )]
    async fn sql_explain(
        &self,
        Parameters(request): Parameters<ExplainRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("sql:read").map_err(invalid)?;
        let validated = validate_explain(request).map_err(invalid)?;
        let allowed_schemas = current_identity()
            .map(|identity| identity.schemas)
            .unwrap_or_default();
        validate_schema_scope(&validated.sql, &allowed_schemas).map_err(invalid)?;
        self.execute_query(QueryRequest {
            sql: validated.sql,
            parameters: validated.parameters,
            row_limit: validated.row_limit.min(self.config.max_rows),
            timeout: validated.timeout,
            max_result_bytes: self.config.max_result_bytes,
            allowed_schemas,
        })
        .await
    }

    #[tool(
        name = "plomid.schema.list_databases",
        description = "List databases visible to the configured PLOMID identity."
    )]
    async fn list_databases(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("schema:read").map_err(invalid)?;
        structured(self.database.list_databases().await.map_err(backend)?)
    }

    #[tool(
        name = "plomid.schema.list_schemas",
        description = "List schemas visible to the configured PLOMID identity."
    )]
    async fn list_schemas(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("schema:read").map_err(invalid)?;
        let schemas = self.database.list_schemas().await.map_err(backend)?;
        structured(filter_schemas(schemas))
    }

    #[tool(
        name = "plomid.schema.list_tables",
        description = "List tables in a schema, defaulting to public."
    )]
    async fn list_tables(
        &self,
        Parameters(request): Parameters<SchemaRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("schema:read").map_err(invalid)?;
        enforce_schema_access(request.schema.as_deref().unwrap_or("public"))?;
        structured(
            self.database
                .list_tables(request.schema)
                .await
                .map_err(backend)?,
        )
    }

    #[tool(
        name = "plomid.schema.describe_table",
        description = "Describe columns and nullability for a visible table."
    )]
    async fn describe_table(
        &self,
        Parameters(request): Parameters<DescribeTableRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("schema:read").map_err(invalid)?;
        if request.schema.is_empty() || request.table.is_empty() {
            return Err(invalid("schema and table are required"));
        }
        enforce_schema_access(&request.schema)?;
        structured(
            self.database
                .describe_table(request.schema, request.table)
                .await
                .map_err(backend)?,
        )
    }

    #[tool(
        name = "plomid.sql.execute",
        description = "Execute one explicitly approved DML statement in controlled-write mode with an idempotency key. Disabled unless server write policy and database privileges both allow it."
    )]
    async fn sql_execute(
        &self,
        Parameters(request): Parameters<WriteRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        AuthPolicy::authorize("sql:write").map_err(invalid)?;
        if self.config.mode != OperatingMode::ControlledWrite || !self.config.write_enabled {
            return Err(invalid(
                "controlled writes are disabled by server configuration",
            ));
        }
        let expected = self
            .config
            .write_approval_token_sha256
            .as_deref()
            .ok_or_else(|| invalid("write approval policy is not configured"))?;
        if !AuthPolicy::verify_token_digest(&request.approval_token, expected) {
            return Err(invalid("write approval token is invalid"));
        }
        let validated = validate_write(request).map_err(invalid)?;
        let allowed_schemas = current_identity()
            .map(|identity| identity.schemas)
            .unwrap_or_default();
        validate_schema_scope(&validated.sql, &allowed_schemas).map_err(invalid)?;
        if !self
            .claim_write(
                &validated.idempotency_key,
                std::time::Duration::from_secs(self.config.write_approval_ttl_secs),
            )
            .await
        {
            return Err(invalid(
                "idempotency key has already been used or is still active",
            ));
        }
        let result = self
            .database
            .execute(ExecuteRequest {
                sql: validated.sql,
                parameters: validated.parameters,
                timeout: validated.timeout,
                allowed_schemas,
            })
            .await
            .map_err(backend)?;
        structured(ExecuteResponse {
            schema_version: "1",
            committed: result.committed,
            data: result,
        })
    }
}

fn filter_schemas(schemas: Vec<String>) -> Vec<String> {
    let Some(identity) = current_identity() else {
        return schemas;
    };
    if identity.schemas.is_empty() {
        return schemas;
    }
    schemas
        .into_iter()
        .filter(|schema| identity.schemas.iter().any(|allowed| allowed == schema))
        .collect()
}

fn enforce_schema_access(schema: &str) -> Result<(), rmcp::ErrorData> {
    let Some(identity) = current_identity() else {
        return Ok(());
    };
    if identity.schemas.is_empty() || identity.schemas.iter().any(|allowed| allowed == schema) {
        Ok(())
    } else {
        Err(invalid("schema is outside the authenticated tenant policy"))
    }
}

impl McpServer {
    async fn execute_query(
        &self,
        request: QueryRequest,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let started = Instant::now();
        let result = self.database.query(request).await.map_err(backend)?;
        let truncated = result.truncated;
        let warnings = result.warnings.clone();
        structured(QueryResponse {
            schema_version: "1",
            data: result,
            meta: QueryMeta {
                request_id: format!(
                    "plomid-{}",
                    REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ),
                elapsed_ms: started.elapsed().as_millis(),
                truncated,
                warnings: [
                    if truncated {
                        vec!["result was bounded by configured row or byte limits".into()]
                    } else {
                        Vec::new()
                    },
                    warnings,
                ]
                .concat(),
                retryable: false,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::backend;

    #[test]
    fn backend_errors_do_not_return_backend_supplied_details() {
        let error = backend("password=synthetic-secret; SQL: SELECT * FROM private.customers");
        assert_eq!(
            error.message,
            "BACKEND_UNAVAILABLE: database operation failed"
        );
        assert!(!error.message.contains("synthetic-secret"));
        assert!(!error.message.contains("private.customers"));
    }
}
