use std::{env, time::Duration};

use plomid_mcp::{
    config::SecretUrl,
    database::{PlomidDatabase, PlomidSqlDatabase, QueryRequest},
};

fn test_database(variable: &str) -> PlomidSqlDatabase {
    let url = env::var(variable).unwrap_or_else(|_| panic!("{variable} must be set for this test"));
    let parsed = SecretUrl::parse(&url).expect("valid PostgreSQL-wire URL");
    PlomidSqlDatabase::new(&parsed, 2, 2).expect("database pool")
}

fn query(sql: &str, parameters: Vec<String>, row_limit: u32) -> QueryRequest {
    QueryRequest {
        sql: sql.into(),
        parameters,
        row_limit,
        timeout: Duration::from_secs(5),
        max_result_bytes: 64 * 1024,
        allowed_schemas: Vec::new(),
    }
}

#[tokio::test]
#[ignore = "requires an explicitly authorized disposable PLOMID endpoint via PLOMID_TEST_DATABASE_URL"]
async fn live_plomid_read_query_parameters_limits_and_errors() {
    let database = test_database("PLOMID_TEST_DATABASE_URL");
    let health = database.health().await.expect("live endpoint health");
    assert!(health.connected);
    assert!(health.server_version.is_some());

    let result = database
        .query(query("SELECT $1::integer AS answer", vec!["42".into()], 4))
        .await
        .expect("parameterized safe query");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], serde_json::json!(42));

    let bounded = database
        .query(query(
            "SELECT 1 AS n UNION ALL SELECT 2 UNION ALL SELECT 3",
            Vec::new(),
            2,
        ))
        .await
        .expect("bounded safe query");
    assert_eq!(bounded.rows.len(), 2);
    assert!(bounded.truncated);

    let empty = database
        .query(query("SELECT 1 AS answer WHERE false", Vec::new(), 4))
        .await
        .expect("empty safe query");
    assert!(empty.rows.is_empty());
    assert_eq!(empty.columns, vec!["answer"]);

    assert!(
        database
            .query(query("SELECT FROM", Vec::new(), 1))
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PLOMID_TEST_TLS_DATABASE_URL and a verified CA in SSL_CERT_FILE"]
async fn live_plomid_tls_certificate_is_verified() {
    let database = test_database("PLOMID_TEST_TLS_DATABASE_URL");
    let health = database.health().await.expect("TLS endpoint health");
    assert!(health.connected);

    let result = database
        .query(query("SELECT $1::integer AS answer", vec!["42".into()], 4))
        .await
        .expect("TLS parameterized query");
    assert_eq!(result.rows, vec![vec![serde_json::json!(42)]]);

    let empty = database
        .query(query("SELECT 1 AS answer WHERE false", Vec::new(), 4))
        .await
        .expect("TLS empty query");
    assert!(empty.rows.is_empty());
    assert_eq!(empty.columns, vec!["answer"]);

    assert!(
        database
            .query(query("SELECT FROM", Vec::new(), 1))
            .await
            .is_err()
    );
}
