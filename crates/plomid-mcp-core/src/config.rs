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
// Validated runtime configuration, capability advertisement, and
// secure precedence rules for local and enterprise PLOMID-MCP deployments.

use std::{net::SocketAddr, path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use url::Url;

use plomid_security::{AuthFileConfig, AuthPolicy};

use crate::cli::Cli;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: Option<SecretUrl>,
    pub mode: OperatingMode,
    pub log_level: String,
    pub bind: SocketAddr,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub max_request_body_bytes: usize,
    pub max_rows: u32,
    pub max_result_bytes: usize,
    pub pool_size: usize,
    pub max_concurrent_queries: usize,
    pub auth: AuthPolicy,
    pub http_concurrency: usize,
    pub rate_limit_per_minute: u32,
    pub write_enabled: bool,
    pub write_approval_token_sha256: Option<String>,
    pub write_approval_ttl_secs: u64,
    pub tls_cert: Option<std::path::PathBuf>,
    pub tls_key: Option<std::path::PathBuf>,
}

#[derive(Clone)]
pub struct SecretUrl(Url);

impl std::fmt::Debug for SecretUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SecretUrl")
            .field(&self.redacted())
            .finish()
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    database_url: Option<String>,
    mode: Option<String>,
    log_level: Option<String>,
    bind: Option<String>,
    allowed_hosts: Option<Vec<String>>,
    allowed_origins: Option<Vec<String>>,
    max_request_body_bytes: Option<usize>,
    max_rows: Option<u32>,
    max_result_bytes: Option<usize>,
    pool_size: Option<usize>,
    max_concurrent_queries: Option<usize>,
    auth: Option<AuthFileConfig>,
    http_concurrency: Option<usize>,
    rate_limit_per_minute: Option<u32>,
    write_enabled: Option<bool>,
    write_approval_token_sha256: Option<String>,
    write_approval_ttl_secs: Option<u64>,
    tls_cert: Option<std::path::PathBuf>,
    tls_key: Option<std::path::PathBuf>,
}

impl SecretUrl {
    pub fn parse(value: &str) -> Result<Self> {
        let url = Url::parse(value).context("database URL is not a valid URL")?;
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            bail!("database URL must use postgres:// or postgresql://");
        }
        if url.host_str().is_none() {
            bail!("database URL must include a host");
        }
        Ok(Self(url))
    }

    pub fn as_url(&self) -> &Url {
        &self.0
    }

    fn redacted(&self) -> String {
        let mut url = self.0.clone();
        if url.password().is_some() {
            let _ = url.set_password(Some("REDACTED"));
        }
        url.to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatingMode {
    ReadOnly,
    ControlledWrite,
}

impl OperatingMode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "read-only" => Ok(Self::ReadOnly),
            "controlled-write" => Ok(Self::ControlledWrite),
            other => bail!("unsupported mode `{other}`; use read-only or controlled-write"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub schema_version: &'static str,
    pub server_mode: &'static str,
    pub database_configured: bool,
    pub database_integration: &'static str,
    pub tools: Vec<&'static str>,
    pub prompts: Vec<&'static str>,
    pub limits: CapabilityLimits,
    pub feature_domains: Vec<FeatureDomain>,
}

#[derive(Debug, Serialize)]
pub struct CapabilityLimits {
    pub max_sql_bytes: usize,
    pub max_parameters: usize,
    pub max_parameter_bytes: usize,
    pub max_rows: u32,
    pub max_result_bytes: usize,
    pub max_concurrent_queries: usize,
    pub max_timeout_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct FeatureDomain {
    pub name: &'static str,
    pub support_status: &'static str,
    pub access: &'static str,
    pub interfaces: Vec<&'static str>,
    pub documented_features: Vec<&'static str>,
    pub limitations: Vec<&'static str>,
}

impl Config {
    pub fn from_cli(cli: &Cli) -> Result<Self> {
        let file = cli
            .settings
            .config
            .as_deref()
            .map(load_file)
            .transpose()?
            .unwrap_or_default();
        let database_url = cli.settings.database_url.clone().or(file.database_url);
        let mode = cli
            .settings
            .mode
            .clone()
            .or(file.mode)
            .unwrap_or_else(|| "read-only".into());
        let log_level = cli
            .settings
            .log_level
            .clone()
            .or(file.log_level)
            .unwrap_or_else(|| "info".into());
        let bind = cli
            .settings
            .bind
            .clone()
            .or(file.bind)
            .unwrap_or_else(|| "127.0.0.1:8080".into());
        let bind: SocketAddr = bind.parse().context("HTTP bind address is invalid")?;
        let auth = AuthPolicy::from_file(file.auth.unwrap_or_default(), !bind.ip().is_loopback())?;
        let config = Self {
            database_url: database_url.as_deref().map(SecretUrl::parse).transpose()?,
            mode: OperatingMode::parse(&mode)?,
            log_level,
            bind,
            allowed_hosts: cli
                .settings
                .allowed_hosts
                .clone()
                .or(file.allowed_hosts)
                .unwrap_or_else(default_allowed_hosts),
            allowed_origins: cli
                .settings
                .allowed_origins
                .clone()
                .or(file.allowed_origins)
                .unwrap_or_else(default_allowed_origins),
            max_request_body_bytes: cli
                .settings
                .max_request_body_bytes
                .or(file.max_request_body_bytes)
                .unwrap_or(4 * 1024 * 1024),
            max_rows: cli.settings.max_rows.or(file.max_rows).unwrap_or(1_000),
            max_result_bytes: cli
                .settings
                .max_result_bytes
                .or(file.max_result_bytes)
                .unwrap_or(8 * 1024 * 1024),
            pool_size: cli.settings.pool_size.or(file.pool_size).unwrap_or(16),
            max_concurrent_queries: cli
                .settings
                .max_concurrent_queries
                .or(file.max_concurrent_queries)
                .unwrap_or(16),
            auth,
            http_concurrency: cli
                .settings
                .http_concurrency
                .or(file.http_concurrency)
                .unwrap_or(64),
            rate_limit_per_minute: cli
                .settings
                .rate_limit_per_minute
                .or(file.rate_limit_per_minute)
                .unwrap_or(600),
            write_enabled: cli
                .settings
                .write_enabled
                .or(file.write_enabled)
                .unwrap_or(false),
            write_approval_token_sha256: cli
                .settings
                .write_approval_token_sha256
                .clone()
                .or(file.write_approval_token_sha256),
            write_approval_ttl_secs: cli
                .settings
                .write_approval_ttl_secs
                .or(file.write_approval_ttl_secs)
                .unwrap_or(300),
            tls_cert: cli.settings.tls_cert.clone().or(file.tls_cert),
            tls_key: cli.settings.tls_key.clone().or(file.tls_key),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.mode == OperatingMode::ReadOnly && self.database_url.is_none() {
            bail!("read-only mode requires PLOMID_DATABASE_URL or --database-url");
        }
        if self.max_request_body_bytes == 0 || self.max_request_body_bytes > 64 * 1024 * 1024 {
            bail!("HTTP request body limit must be between 1 byte and 64 MiB");
        }
        if self.allowed_hosts.is_empty() {
            bail!("at least one allowed HTTP host is required");
        }
        if self.max_rows == 0 || self.max_rows > 100_000 {
            bail!("max rows must be between 1 and 100000");
        }
        if self.max_result_bytes == 0 || self.max_result_bytes > 256 * 1024 * 1024 {
            bail!("max result bytes must be between 1 and 256 MiB");
        }
        if self.pool_size == 0 || self.pool_size > 256 {
            bail!("pool size must be between 1 and 256");
        }
        if self.max_concurrent_queries == 0 || self.max_concurrent_queries > 256 {
            bail!("maximum concurrent queries must be between 1 and 256");
        }
        if self.http_concurrency == 0 || self.http_concurrency > 4096 {
            bail!("HTTP concurrency must be between 1 and 4096");
        }
        if self.rate_limit_per_minute == 0 || self.rate_limit_per_minute > 1_000_000 {
            bail!("HTTP rate limit must be between 1 and 1000000 per minute");
        }
        if self.write_enabled {
            if self.mode != OperatingMode::ControlledWrite {
                bail!("write_enabled requires controlled-write mode");
            }
            let Some(digest) = &self.write_approval_token_sha256 else {
                bail!("controlled writes require a write approval token digest");
            };
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("write approval token digest must be a SHA-256 hexadecimal digest");
            }
        }
        if self.tls_cert.is_some() != self.tls_key.is_some() {
            bail!("TLS certificate and key must be configured together");
        }
        if !self.bind.ip().is_loopback() && self.tls_cert.is_none() {
            bail!("non-loopback HTTP requires direct TLS certificate/key configuration");
        }
        Ok(())
    }

    pub fn init_logging(&self) -> Result<()> {
        let filter = format!("{},rmcp=off", self.log_level);
        let stderr = std::io::stderr();
        let terminal = std::io::IsTerminal::is_terminal(&stderr);
        let ansi = terminal && std::env::var_os("NO_COLOR").is_none();

        // Keep terminal output human-readable (and colored when enabled) while
        // preserving structured JSON logs for redirected and service output.
        // rmcp logs client-controlled peer details, so that target stays off.
        let result = if terminal {
            tracing_subscriber::fmt()
                .pretty()
                .with_target(false)
                .with_file(false)
                .with_line_number(false)
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .with_ansi(ansi)
                .try_init()
        } else {
            tracing_subscriber::fmt()
                .with_target(false)
                .with_file(false)
                .with_line_number(false)
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .json()
                .with_ansi(false)
                .try_init()
        };
        result.map_err(|error| anyhow::anyhow!("failed to initialize logging: {error}"))
    }

    pub fn capabilities(&self) -> Capabilities {
        let mut tools = vec![
            "plomid.status",
            "plomid.capabilities",
            "plomid.sql.query",
            "plomid.sql.explain",
            "plomid.schema.list_databases",
            "plomid.schema.list_schemas",
            "plomid.schema.list_tables",
            "plomid.schema.describe_table",
        ];
        if self.mode == OperatingMode::ControlledWrite && self.write_enabled {
            tools.push("plomid.sql.execute");
        }
        Capabilities {
            schema_version: "1",
            server_mode: match self.mode {
                OperatingMode::ReadOnly => "read-only",
                OperatingMode::ControlledWrite => "controlled-write",
            },
            database_configured: self.database_url.is_some(),
            database_integration: "pooled PostgreSQL adapter; live connectivity reported by plomid.status",
            tools,
            prompts: vec!["plomid.safe-query-workflow"],
            limits: CapabilityLimits {
                max_sql_bytes: crate::sql::MAX_SQL_BYTES,
                max_parameters: crate::sql::MAX_PARAMETERS,
                max_parameter_bytes: crate::sql::MAX_PARAMETER_BYTES,
                max_rows: self.max_rows,
                max_result_bytes: self.max_result_bytes,
                max_concurrent_queries: self.max_concurrent_queries,
                max_timeout_ms: crate::sql::MAX_TIMEOUT_MS,
            },
            feature_domains: documented_feature_domains(),
        }
    }
}

fn documented_feature_domains() -> Vec<FeatureDomain> {
    vec![
        (
            "relational-sql",
            vec!["SELECT", "joins", "CTE", "windows", "pagination"],
        ),
        (
            "vectors",
            vec![
                "dense vector types",
                "distance operators",
                "top-k",
                "filtered search",
                "HNSW/ANN",
                "vector EXPLAIN",
            ],
        ),
        (
            "json-and-documents",
            vec![
                "JSON/JSONB",
                "JSON_TABLE",
                "path operators",
                "JSON aggregates",
            ],
        ),
        (
            "time-series-and-temporal",
            vec![
                "timestamps",
                "interval arithmetic",
                "EXTRACT",
                "date_trunc",
                "time predicates",
            ],
        ),
        (
            "transactions-and-mvcc",
            vec!["MVCC snapshots", "conflict behavior", "isolation settings"],
        ),
        (
            "metadata-and-catalog",
            vec![
                "pg_catalog",
                "information_schema",
                "SHOW",
                "DESCRIBE",
                "RBAC metadata",
            ],
        ),
        (
            "types-functions-and-indexes",
            vec![
                "arrays",
                "ranges",
                "enums",
                "domains",
                "functions",
                "aggregates",
                "indexes",
            ],
        ),
    ]
    .into_iter()
    .map(|(name, documented_features)| FeatureDomain {
        name,
        support_status: "IMPLEMENTED_READ_ONLY_INTEGRATION_VALIDATED",
        access: "read-only",
        interfaces: vec!["plomid.sql.query", "plomid.sql.explain", "plomid.schema.*"],
        documented_features,
        limitations: vec!["mutations are separately gated by controlled-write policy"],
    })
    .collect()
}

fn default_allowed_hosts() -> Vec<String> {
    vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]
}

fn default_allowed_origins() -> Vec<String> {
    vec![
        "http://localhost:*".into(),
        "http://127.0.0.1:*".into(),
        "http://[::1]:*".into(),
    ]
}

fn load_file(path: &Path) -> Result<FileConfig> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read configuration file {}", path.display()))?;
    toml::from_str(&contents)
        .with_context(|| format!("invalid TOML configuration in {}", path.display()))
}

pub type SharedConfig = Arc<Config>;
