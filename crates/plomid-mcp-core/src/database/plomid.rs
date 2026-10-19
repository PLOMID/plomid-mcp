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
// Production PLOMID PostgreSQL-wire adapter with pooling,
// bounded execution, compatibility parameter substitution, schema isolation,
// metadata discovery, and controlled transactional writes.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use deadpool_postgres::{Config as PoolConfig, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio::{
    sync::Semaphore,
    time::{Instant, timeout_at},
};
use tokio_postgres::{
    Row, SimpleQueryMessage,
    types::{FromSql, ToSql, Type},
};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::{
    ColumnInfo, DatabaseHealth, ExecuteRequest, ExecuteResult, IndexInfo, KeyInfo, PlomidDatabase,
    QueryRequest, QueryResult, TableDescription, TableInfo,
};
use crate::config::SecretUrl;

const DATABASE_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct PlomidSqlDatabase {
    pool: Arc<Pool>,
    query_slots: Arc<Semaphore>,
}

impl PlomidSqlDatabase {
    pub fn new(url: &SecretUrl, max_size: usize, max_concurrent_queries: usize) -> Result<Self> {
        let mut config = PoolConfig::new();
        config.url = Some(url.as_url().to_string());
        config.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        config.pool = Some(deadpool_postgres::PoolConfig {
            max_size,
            ..Default::default()
        });
        // Respect `sslmode` from the PostgreSQL URL. Rustls validates the
        // server certificate against the platform trust store; deployments
        // with a private PLOMID CA can provide it through SSL_CERT_FILE.
        // The database protocol may still use plaintext when the URL
        // explicitly selects `sslmode=disable` or the server declines TLS
        // under its default `prefer` policy.
        // Another TLS server dependency may enable a different provider;
        // select ring when no provider has already been selected.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        // Some minimal/container environments expose no native certificate
        // bundle. Do not make server construction fail in that case (for
        // example, when sslmode=disable); use the maintained WebPKI roots as
        // a secure fallback. A private CA can still be supplied via
        // SSL_CERT_FILE and is loaded by `with_native_certs` when available.
        // TLS verification remains enabled in either path; there is no
        // dangerous-certificate-verifier fallback.
        let tls = MakeRustlsConnect::with_native_certs()
            .map(|(connector, _warnings)| connector)
            .unwrap_or_else(|_| MakeRustlsConnect::with_webpki_roots());
        let pool = config
            .create_pool(Some(Runtime::Tokio1), tls)
            .context("failed to create PLOMID SQL pool")?;
        Ok(Self {
            pool: Arc::new(pool),
            query_slots: Arc::new(Semaphore::new(max_concurrent_queries)),
        })
    }
    async fn client_until(&self, deadline: Instant) -> Result<deadpool_postgres::Object> {
        timeout_at(deadline, self.pool.get())
            .await
            .context("PLOMID connection-pool acquisition timed out")?
            .context("failed to acquire PLOMID SQL connection")
    }
}

#[async_trait::async_trait]
impl PlomidDatabase for PlomidSqlDatabase {
    async fn health(&self) -> Result<DatabaseHealth> {
        let deadline = Instant::now() + DATABASE_CONTROL_TIMEOUT;
        let client = self.client_until(deadline).await?;
        let version: String = timeout_at(deadline, client.query_one("SHOW server_version", &[]))
            .await
            .context("PLOMID health query timed out")??
            .get(0);
        Ok(DatabaseHealth {
            connected: true,
            server_version: Some(version),
        })
    }
    async fn query(&self, request: QueryRequest) -> Result<QueryResult> {
        let deadline = Instant::now() + request.timeout;
        let permit = timeout_at(deadline, self.query_slots.clone().acquire_owned())
            .await
            .context("PLOMID query concurrency wait timed out")??;
        let mut client = self.client_until(deadline).await?;
        let params: Vec<&(dyn ToSql + Sync)> = request
            .parameters
            .iter()
            .map(|value| value as &(dyn ToSql + Sync))
            .collect();
        let transaction = timeout_at(deadline, client.build_transaction().read_only(true).start())
            .await
            .context("starting read-only PLOMID transaction timed out")?
            .context("failed to start read-only PLOMID transaction")?;
        timeout_at(
            deadline,
            set_local_search_path(&transaction, &request.allowed_schemas),
        )
        .await
        .context("applying PLOMID schema policy timed out")??;
        if request.parameters.is_empty() && request.sql.to_ascii_lowercase().contains("vector") {
            return simple_query_result(&transaction, &request, &request.sql, deadline).await;
        }
        if !request.parameters.is_empty() {
            let sql = substitute_parameters(&request.sql, &request.parameters)?;
            return simple_query_result(&transaction, &request, &sql, deadline).await;
        }
        let query_result = timeout_at(deadline, transaction.query(&request.sql, &params))
            .await
            .context("PLOMID query timed out")?;
        let rows = match query_result {
            Ok(rows) => rows,
            Err(error) if error.to_string().contains("expected 0 parameters") => {
                let sql = substitute_parameters(&request.sql, &request.parameters)?;
                return simple_query_result(&transaction, &request, &sql, deadline).await;
            }
            Err(error) => return Err(error.into()),
        };
        let columns: Vec<String> = rows
            .first()
            .map(|row| {
                row.columns()
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let columns = if columns.is_empty() {
            // `query` has no Row value from which to recover metadata when a
            // valid SELECT returns zero rows. Prepare is metadata-only here;
            // if this backend cannot prepare the statement, preserve the
            // successful empty result rather than turning it into an error.
            timeout_at(deadline, transaction.prepare(&request.sql))
                .await
                .ok()
                .and_then(Result::ok)
                .map(|statement| {
                    statement
                        .columns()
                        .iter()
                        .map(|column| column.name().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            columns
        };
        let mut output = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = rows.len() > request.row_limit as usize;
        for row in rows.into_iter().take(request.row_limit as usize) {
            let values = row_to_json(&row)?;
            let encoded = serde_json::to_vec(&values)?;
            if bytes.saturating_add(encoded.len()) > request.max_result_bytes {
                truncated = true;
                break;
            }
            bytes += encoded.len();
            output.push(values);
        }
        drop(permit);
        Ok(QueryResult {
            columns,
            rows: output,
            truncated,
            warnings: Vec::new(),
        })
    }
    async fn execute(&self, request: ExecuteRequest) -> Result<ExecuteResult> {
        let deadline = Instant::now() + request.timeout;
        let permit = timeout_at(deadline, self.query_slots.clone().acquire_owned())
            .await
            .context("PLOMID write concurrency wait timed out")??;
        let mut client = self.client_until(deadline).await?;
        let sql = substitute_parameters(&request.sql, &request.parameters)?;
        let transaction = timeout_at(deadline, client.build_transaction().start())
            .await
            .context("starting controlled-write transaction timed out")?
            .context("failed to start controlled-write transaction")?;
        timeout_at(
            deadline,
            set_local_search_path(&transaction, &request.allowed_schemas),
        )
        .await
        .context("applying controlled-write schema policy timed out")??;
        let messages = timeout_at(deadline, transaction.simple_query(&sql))
            .await
            .context("PLOMID write timed out")??;
        let affected_rows = messages
            .into_iter()
            .filter_map(|message| match message {
                SimpleQueryMessage::CommandComplete(rows) => Some(rows),
                _ => None,
            })
            .last()
            .unwrap_or(0);
        timeout_at(deadline, transaction.commit())
            .await
            .context("controlled-write commit timed out; outcome may be uncertain")?
            .context("controlled-write commit failed; outcome may be uncertain")?;
        drop(permit);
        Ok(ExecuteResult {
            affected_rows,
            committed: true,
        })
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        let deadline = Instant::now() + DATABASE_CONTROL_TIMEOUT;
        let c = self.client_until(deadline).await?;
        Ok(timeout_at(
            deadline,
            c.query(
                "SELECT datname FROM pg_database WHERE datallowconn ORDER BY datname",
                &[],
            ),
        )
        .await
        .context("database listing timed out")??
        .into_iter()
        .map(|r| r.get(0))
        .collect())
    }
    async fn list_schemas(&self) -> Result<Vec<String>> {
        let deadline = Instant::now() + DATABASE_CONTROL_TIMEOUT;
        let c = self.client_until(deadline).await?;
        Ok(timeout_at(
            deadline,
            c.query(
                "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name",
                &[],
            ),
        )
        .await
        .context("schema listing timed out")??
        .into_iter()
        .map(|r| r.get(0))
        .collect())
    }
    async fn list_tables(&self, schema: Option<String>) -> Result<Vec<TableInfo>> {
        let deadline = Instant::now() + DATABASE_CONTROL_TIMEOUT;
        let c = self.client_until(deadline).await?;
        let schema = schema.unwrap_or_else(|| "public".into());
        let sql = format!(
            "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema = {} ORDER BY table_name",
            sql_literal(&schema)
        );
        Ok(timeout_at(deadline, c.query(&sql, &[]))
            .await
            .context("table listing timed out")??
            .into_iter()
            .map(|r| TableInfo {
                schema: r.get(0),
                name: r.get(1),
                kind: r.get(2),
            })
            .collect())
    }
    async fn describe_table(&self, schema: String, table: String) -> Result<TableDescription> {
        let deadline = Instant::now() + DATABASE_CONTROL_TIMEOUT;
        let c = self.client_until(deadline).await?;
        let schema_literal = sql_literal(&schema);
        let table_literal = sql_literal(&table);
        let rows = timeout_at(deadline, c.query(&format!("SELECT table_schema, table_name, column_name, data_type, is_nullable, ordinal_position FROM information_schema.columns WHERE table_schema = {schema_literal} AND table_name = {table_literal} ORDER BY ordinal_position"), &[]))
            .await
            .context("table description timed out")??;
        if rows.is_empty() {
            bail!("table not found: {schema}.{table}");
        }
        let columns = rows
            .iter()
            .map(|r| ColumnInfo {
                name: r.get(2),
                data_type: r.get(3),
                nullable: r.get::<_, String>(4) == "YES",
                ordinal_position: r.get(5),
            })
            .collect();
        let key_rows = timeout_at(deadline, c.query(&format!("SELECT tc.constraint_name, tc.constraint_type, kcu.column_name FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema WHERE tc.table_schema = {schema_literal} AND tc.table_name = {table_literal} ORDER BY tc.constraint_name, kcu.ordinal_position"), &[]))
            .await
            .context("table key lookup timed out")??;
        let mut keys = Vec::<KeyInfo>::new();
        for row in key_rows {
            let name: String = row.get(0);
            let kind: String = row.get(1);
            let column: String = row.get(2);
            if let Some(key) = keys.iter_mut().find(|key| key.name == name) {
                key.columns.push(column);
            } else {
                keys.push(KeyInfo {
                    name,
                    kind,
                    columns: vec![column],
                });
            }
        }
        let indexes = timeout_at(deadline, c.query(&format!("SELECT indexname, indexdef FROM pg_indexes WHERE schemaname = {schema_literal} AND tablename = {table_literal} ORDER BY indexname"), &[]))
            .await
            .context("table index lookup timed out")??
            .into_iter().map(|row| IndexInfo { name: row.get(0), definition: row.get(1) }).collect();
        Ok(TableDescription {
            schema,
            name: table,
            columns,
            keys,
            indexes,
        })
    }
}

async fn set_local_search_path(
    transaction: &tokio_postgres::Transaction<'_>,
    allowed_schemas: &[String],
) -> Result<()> {
    if allowed_schemas.is_empty() {
        return Ok(());
    }
    let search_path = allowed_schemas
        .iter()
        .map(|schema| quote_identifier(schema))
        .collect::<Vec<_>>()
        .join(", ");
    transaction
        .batch_execute(&format!(
            "SET LOCAL search_path TO {search_path}, pg_catalog"
        ))
        .await
        .context("failed to apply request schema policy")?;
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn simple_query_result(
    transaction: &tokio_postgres::Transaction<'_>,
    request: &QueryRequest,
    sql: &str,
    deadline: Instant,
) -> Result<QueryResult> {
    let messages = timeout_at(deadline, transaction.simple_query(sql))
        .await
        .context("PLOMID simple query timed out")??;
    let mut columns = Vec::new();
    let mut output = Vec::new();
    let mut truncated = false;
    let mut bytes = 0usize;
    for message in messages {
        match message {
            SimpleQueryMessage::Row(row) => {
                if output.len() >= request.row_limit as usize {
                    truncated = true;
                    break;
                }
                let values = (0..row.len())
                    .map(|index| parse_simple_value(row.try_get(index).ok().flatten()))
                    .collect::<Vec<_>>();
                let encoded = serde_json::to_vec(&values)?;
                if bytes.saturating_add(encoded.len()) > request.max_result_bytes {
                    truncated = true;
                    break;
                }
                bytes += encoded.len();
                output.push(values);
            }
            SimpleQueryMessage::RowDescription(description) if columns.is_empty() => {
                columns = description
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect();
            }
            SimpleQueryMessage::RowDescription(_) | SimpleQueryMessage::CommandComplete(_) => {}
            _ => {}
        }
    }
    Ok(QueryResult {
        columns,
        rows: output,
        truncated,
        warnings: if request.parameters.is_empty() {
            Vec::new()
        } else {
            vec!["PLOMID SQL compatibility mode safely substituted parameters because the endpoint does not support SQL placeholders".into()]
        },
    })
}

fn substitute_parameters(sql: &str, parameters: &[String]) -> Result<String> {
    let mut output = String::with_capacity(sql.len());
    let mut index = 0;
    let bytes = sql.as_bytes();
    let mut quote = None;
    let mut dollar_quote: Option<String> = None;
    let mut line_comment = false;
    let mut block_comment = 0_u32;
    let mut used = vec![false; parameters.len()];
    while index < bytes.len() {
        let character = sql[index..]
            .chars()
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid SQL parameter boundary"))?;
        let size = character.len_utf8();
        if line_comment {
            output.push(character);
            index += size;
            if character == '\n' {
                line_comment = false;
            }
            continue;
        }
        if block_comment > 0 {
            if index + 1 < bytes.len() && &sql[index..index + 2] == "/*" {
                output.push_str("/*");
                index += 2;
                block_comment += 1;
                continue;
            }
            if index + 1 < bytes.len() && &sql[index..index + 2] == "*/" {
                output.push_str("*/");
                index += 2;
                block_comment -= 1;
                continue;
            }
            output.push(character);
            index += size;
            continue;
        }
        if let Some(delimiter) = &dollar_quote {
            if sql[index..].starts_with(delimiter) {
                output.push_str(delimiter);
                index += delimiter.len();
                dollar_quote = None;
            } else {
                output.push(character);
                index += size;
            }
            continue;
        }
        if let Some(expected) = quote {
            output.push(character);
            if character == expected {
                if index + size < bytes.len() && bytes[index + size] == expected as u8 {
                    output.push(expected);
                    index += size * 2;
                    continue;
                }
                quote = None;
            }
            index += size;
            continue;
        }
        if sql[index..].starts_with("--") {
            output.push_str("--");
            index += 2;
            line_comment = true;
            continue;
        }
        if sql[index..].starts_with("/*") {
            output.push_str("/*");
            index += 2;
            block_comment = 1;
            continue;
        }
        if character == '$'
            && let Some(end) = sql[index + 1..].find('$')
        {
            let delimiter = &sql[index..=index + end + 1];
            if delimiter[1..delimiter.len() - 1]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                output.push_str(delimiter);
                index += delimiter.len();
                dollar_quote = Some(delimiter.to_owned());
                continue;
            }
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
            output.push(character);
            index += size;
            continue;
        }
        if character == '$' && index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let number: usize = sql[index + 1..end]
                .parse()
                .context("invalid SQL parameter number")?;
            let value = parameters
                .get(number.saturating_sub(1))
                .ok_or_else(|| anyhow::anyhow!("SQL parameter ${number} has no supplied value"))?;
            used[number - 1] = true;
            output.push('\'');
            output.push_str(&value.replace('\'', "''"));
            output.push('\'');
            index = end;
            continue;
        }
        output.push(character);
        index += size;
    }
    if quote.is_some() || dollar_quote.is_some() || block_comment > 0 {
        bail!("unterminated SQL quote or comment");
    }
    if used.iter().any(|was_used| !was_used) {
        bail!("SQL parameter list contains an unused value");
    }
    Ok(output)
}

fn parse_simple_value(value: Option<&str>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::Value::Null;
    };
    if (value.starts_with('[') || value.starts_with('{') || value == "true" || value == "false")
        && let Ok(parsed) = serde_json::from_str(value)
    {
        return parsed;
    }
    if let Ok(parsed) = value.parse::<i64>() {
        return serde_json::json!(parsed);
    }
    if let Ok(parsed) = value.parse::<f64>() {
        return serde_json::json!(parsed);
    }
    serde_json::Value::String(value.to_owned())
}

fn row_to_json(row: &Row) -> Result<Vec<serde_json::Value>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let value = match column.type_().name() {
                "bool" => row
                    .try_get::<_, Option<bool>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, serde_json::Value::Bool))
                    .context("failed to decode boolean")?,
                "int2" => row
                    .try_get::<_, Option<i16>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode int2")?,
                "int4" => row
                    .try_get::<_, Option<i32>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode int4")?,
                "int8" => row
                    .try_get::<_, Option<i64>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode int8")?,
                "float4" | "float8" | "numeric" => row
                    .try_get::<_, Option<f64>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode number")?,
                "date" => row
                    .try_get::<_, Option<chrono::NaiveDate>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode date")?,
                "timestamp" => row
                    .try_get::<_, Option<chrono::NaiveDateTime>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode timestamp")?,
                "timestamptz" => row
                    .try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode timestamptz")?,
                "time" => row
                    .try_get::<_, Option<chrono::NaiveTime>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode time")?,
                "text[]" | "_text" => row
                    .try_get::<_, Option<Vec<String>>>(index)
                    .map(|v| v.map_or(serde_json::Value::Null, |v| serde_json::json!(v)))
                    .context("failed to decode text array")?,
                "json" | "jsonb" => {
                    let value: Option<String> =
                        row.try_get(index).context("failed to decode JSON")?;
                    value.map_or(Ok(serde_json::Value::Null), |value| {
                        serde_json::from_str(&value).context("failed to parse JSON")
                    })?
                }
                _ => row
                    .try_get::<_, Option<RawPLOMIDValue>>(index)
                    .map(|v| {
                        v.map_or(serde_json::Value::Null, |value| {
                            serde_json::Value::String(value.0)
                        })
                    })
                    .context("failed to decode PLOMID value")?,
            };
            Ok(value)
        })
        .collect()
}

struct RawPLOMIDValue(String);

impl<'a> FromSql<'a> for RawPLOMIDValue {
    fn accepts(_ty: &Type) -> bool {
        true
    }

    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        match std::str::from_utf8(raw) {
            Ok(value) => Ok(Self(value.to_owned())),
            Err(_) => Ok(Self(raw.iter().map(|byte| format!("{byte:02x}")).collect())),
        }
    }

    fn from_sql_null(_ty: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(String::new()))
    }
}

#[cfg(test)]
mod parameter_tests {
    use super::substitute_parameters;

    #[test]
    fn substitutes_only_real_parameters() {
        let sql = "SELECT '$1', $$ $2 $$, $1 /* $2 */ -- $2\n";
        assert_eq!(
            substitute_parameters(sql, &["safe'".into()]).unwrap(),
            "SELECT '$1', $$ $2 $$, 'safe''' /* $2 */ -- $2\n"
        );
    }

    #[test]
    fn rejects_unused_parameters() {
        assert!(substitute_parameters("SELECT 1", &["unused".into()]).is_err());
        assert!(substitute_parameters("SELECT $2", &["one".into(), "two".into()]).is_err());
    }
}
