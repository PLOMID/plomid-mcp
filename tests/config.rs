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
// Configuration regression tests covering safety defaults,
// secret redaction, validation, and capability advertisement.

use plomid_mcp::config::{Config, OperatingMode, SecretUrl};
use plomid_security::AuthPolicy;
use tempfile::tempdir;

fn config(database_url: Option<SecretUrl>) -> Config {
    Config {
        database_url,
        mode: OperatingMode::ReadOnly,
        log_level: "info".to_string(),
        bind: "127.0.0.1:8080".parse().unwrap(),
        allowed_hosts: vec!["localhost".to_string()],
        allowed_origins: Vec::new(),
        max_request_body_bytes: 4 * 1024 * 1024,
        max_rows: 1_000,
        max_result_bytes: 8 * 1024 * 1024,
        pool_size: 16,
        max_concurrent_queries: 16,
        auth: AuthPolicy::from_file(Default::default(), false).unwrap(),
        http_concurrency: 64,
        rate_limit_per_minute: 600,
        write_enabled: false,
        write_approval_token_sha256: None,
        write_approval_ttl_secs: 300,
        tls_cert: None,
        tls_key: None,
    }
}

#[test]
fn read_only_requires_database_url() {
    assert!(config(None).validate().is_err());
}

#[test]
fn validates_database_url_scheme_without_exposing_password() {
    let url = SecretUrl::parse("postgresql://reader:secret@localhost:5432/plomid").expect("url");
    assert_eq!(url.as_url().scheme(), "postgresql");
    assert!(!format!("{url:?}").contains("secret"));
}

#[test]
fn validates_http_limits_and_hosts() {
    let directory = tempdir().expect("tempdir");
    let mut value = config(Some(
        SecretUrl::parse("postgresql://reader@localhost/plomid").unwrap(),
    ));
    value.max_request_body_bytes = 0;
    assert!(value.validate().is_err());
    value.max_request_body_bytes = 4 * 1024 * 1024;
    value.allowed_hosts.clear();
    assert!(value.validate().is_err());
    assert!(directory.path().exists());
}

#[test]
fn capabilities_expose_database_tools_only() {
    let value = config(Some(
        SecretUrl::parse("postgresql://reader@localhost/plomid").unwrap(),
    ));
    let capabilities = value.capabilities();
    assert!(capabilities.tools.contains(&"plomid.sql.query"));
    assert!(capabilities.tools.contains(&"plomid.sql.explain"));
    assert_eq!(capabilities.prompts, vec!["plomid.safe-query-workflow"]);
    assert!(
        !capabilities
            .tools
            .iter()
            .any(|tool| tool.starts_with("plomid.docs."))
    );
}

#[test]
fn controlled_write_capability_is_not_advertised_by_mode_alone() {
    let mut value = config(Some(
        SecretUrl::parse("postgresql://writer@localhost/plomid").unwrap(),
    ));
    value.mode = OperatingMode::ControlledWrite;
    assert!(!value.capabilities().tools.contains(&"plomid.sql.execute"));
    value.write_enabled = true;
    value.write_approval_token_sha256 = Some("a".repeat(64));
    assert!(value.validate().is_ok());
    assert!(value.capabilities().tools.contains(&"plomid.sql.execute"));
}
