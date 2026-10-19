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
// Conservative SQL classification, limits, parameter checks, read-only
// enforcement, and controlled-write validation.
use super::{
    ExplainRequest, MAX_PARAMETER_BYTES, MAX_PARAMETERS, MAX_ROW_LIMIT, MAX_SQL_BYTES,
    MAX_TIMEOUT_MS, SqlRequest, StatementClass, ValidatedSql, ValidatedWrite, WriteRequest,
};
use anyhow::{Result, bail};
use sqlparser::{
    ast::{ObjectName, ObjectNamePart, Statement, Visit, Visitor},
    dialect::PostgreSqlDialect,
    parser::Parser,
};
use std::ops::ControlFlow;
use std::time::Duration;

/// Enforce the request's schema scope for statically named relations. The
/// database role remains the security boundary for indirect access through
/// functions, views, and backend-specific features.
pub fn validate_schema_scope(sql: &str, allowed_schemas: &[String]) -> Result<()> {
    if allowed_schemas.is_empty() {
        return Ok(());
    }

    let statements = Parser::parse_sql(&PostgreSqlDialect {}, sql)
        .map_err(|_| anyhow::anyhow!("schema-scoped SQL could not be safely analyzed"))?;
    if statements.len() != 1 {
        bail!("schema-scoped SQL must contain exactly one statement");
    }

    let mut checker = SchemaScopeChecker {
        allowed_schemas,
        violation: None,
    };
    let _ = statements.visit(&mut checker);
    if let Some(reason) = checker.violation {
        bail!("{reason}");
    }
    Ok(())
}

struct SchemaScopeChecker<'a> {
    allowed_schemas: &'a [String],
    violation: Option<String>,
}

impl Visitor for SchemaScopeChecker<'_> {
    type Break = ();

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        if relation.0.len() == 1 {
            return ControlFlow::Continue(());
        }
        if relation.0.len() != 2 {
            self.violation = Some("database-qualified relations are outside schema policy".into());
            return ControlFlow::Break(());
        }
        let Some(schema) = relation.0[0].as_ident() else {
            self.violation = Some("dynamic schema names are not allowed in scoped SQL".into());
            return ControlFlow::Break(());
        };
        let schema_name = if schema.quote_style.is_none() {
            schema.value.to_ascii_lowercase()
        } else {
            schema.value.clone()
        };
        if !self
            .allowed_schemas
            .iter()
            .any(|allowed| allowed == &schema_name)
        {
            self.violation = Some("SQL relation is outside the authenticated schema policy".into());
            return ControlFlow::Break(());
        }
        if relation
            .0
            .iter()
            .any(|part| matches!(part, ObjectNamePart::Function(_)))
        {
            self.violation = Some("dynamic relation names are not allowed in scoped SQL".into());
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

pub fn validate_read_only(request: SqlRequest) -> Result<ValidatedSql> {
    if request.sql.len() > MAX_SQL_BYTES {
        bail!("SQL exceeds the {} byte limit", MAX_SQL_BYTES);
    }
    if request.sql.trim().is_empty() {
        bail!("SQL must not be empty");
    }
    if request.parameters.len() > MAX_PARAMETERS {
        bail!("too many parameters; maximum is {}", MAX_PARAMETERS);
    }
    if request
        .parameters
        .iter()
        .any(|value| value.len() > MAX_PARAMETER_BYTES)
    {
        bail!("a parameter exceeds the {} byte limit", MAX_PARAMETER_BYTES);
    }
    let statements = split_statements(&request.sql)?;
    if statements.len() != 1 {
        bail!("exactly one SQL statement is required");
    }
    let statement_class = classify_statement(statements[0]);
    match statement_class {
        StatementClass::Read | StatementClass::Explain | StatementClass::SessionRead => {}
        _ => bail!(
            "statement is not permitted in read-only mode: {:?}",
            statement_class
        ),
    }
    if matches!(
        statement_class,
        StatementClass::Read | StatementClass::Explain
    ) && contains_data_modifying_sql(statements[0])?
    {
        bail!("data-modifying SQL is not permitted in read-only mode");
    }
    Ok(ValidatedSql {
        sql: statements[0].trim().to_string(),
        parameters: request.parameters,
        row_limit: request.row_limit.clamp(1, MAX_ROW_LIMIT),
        timeout: Duration::from_millis(request.timeout_ms.clamp(1, MAX_TIMEOUT_MS)),
        statement_class,
    })
}

fn contains_data_modifying_sql(sql: &str) -> Result<bool> {
    if !lexical_words(sql)
        .first()
        .is_some_and(|word| word == "WITH" || word == "EXPLAIN")
    {
        return Ok(false);
    }
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, sql)
        .map_err(|_| anyhow::anyhow!("read-only SQL could not be safely analyzed"))?;
    let mut checker = DataModificationChecker { found: false };
    let _ = statements.visit(&mut checker);
    Ok(checker.found)
}

struct DataModificationChecker {
    found: bool,
}

impl Visitor for DataModificationChecker {
    type Break = ();

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<Self::Break> {
        if matches!(
            statement,
            Statement::Insert(_)
                | Statement::Update(_)
                | Statement::Delete(_)
                | Statement::Merge(_)
        ) {
            self.found = true;
            return ControlFlow::Break(());
        }
        if let Statement::Explain { statement, .. } = statement
            && matches!(
                statement.as_ref(),
                Statement::Insert(_)
                    | Statement::Update(_)
                    | Statement::Delete(_)
                    | Statement::Merge(_)
            )
        {
            self.found = true;
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

pub fn validate_explain(request: ExplainRequest) -> Result<ValidatedSql> {
    let validated = validate_read_only(SqlRequest {
        sql: request.sql,
        parameters: request.parameters,
        row_limit: request.row_limit,
        timeout_ms: request.timeout_ms,
    })?;
    if validated.statement_class != StatementClass::Explain {
        bail!("plomid.sql.explain requires an EXPLAIN statement");
    }
    Ok(validated)
}

pub fn validate_write(request: WriteRequest) -> Result<ValidatedWrite> {
    if request.sql.len() > MAX_SQL_BYTES {
        bail!("SQL exceeds the {} byte limit", MAX_SQL_BYTES);
    }
    if request.parameters.len() > MAX_PARAMETERS {
        bail!("too many parameters; maximum is {}", MAX_PARAMETERS);
    }
    if request
        .parameters
        .iter()
        .any(|value| value.len() > MAX_PARAMETER_BYTES)
    {
        bail!("a parameter exceeds the {} byte limit", MAX_PARAMETER_BYTES);
    }
    if request.approval_token.is_empty() {
        bail!("approval_token is required");
    }
    if request.idempotency_key.len() < 16 || request.idempotency_key.len() > 256 {
        bail!("idempotency_key must contain 16 to 256 characters");
    }
    let statements = split_statements(&request.sql)?;
    if statements.len() != 1 {
        bail!("exactly one SQL statement is required");
    }
    if !matches!(classify_statement(statements[0]), StatementClass::Mutation) {
        bail!("only DML statements are permitted by controlled-write mode");
    }
    Ok(ValidatedWrite {
        sql: statements[0].trim().to_string(),
        parameters: request.parameters,
        timeout: Duration::from_millis(request.timeout_ms.clamp(1, MAX_TIMEOUT_MS)),
        idempotency_key: request.idempotency_key,
        approval_token: request.approval_token,
    })
}

fn classify_statement(statement: &str) -> StatementClass {
    match lexical_words(statement)
        .first()
        .map(String::as_str)
        .unwrap_or_default()
    {
        "SELECT" | "WITH" | "VALUES" => StatementClass::Read,
        "EXPLAIN" => StatementClass::Explain,
        "SHOW" | "DESCRIBE" | "USE" | "SET" => StatementClass::SessionRead,
        "BEGIN" | "START" | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" => {
            StatementClass::Transaction
        }
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "TRUNCATE" | "COPY" => StatementClass::Mutation,
        "CREATE" | "ALTER" | "DROP" | "COMMENT" | "GRANT" | "REVOKE" => {
            StatementClass::Administrative
        }
        _ => StatementClass::Unknown,
    }
}

fn lexical_words(value: &str) -> Vec<String> {
    value
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .map(|word| word.to_ascii_uppercase())
        .collect()
}

fn split_statements(value: &str) -> Result<Vec<&str>> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut dollar_quote: Option<String> = None;
    let mut line_comment = false;
    let mut block_comment = 0_u32;
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let character = value[index..]
            .chars()
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid SQL boundary"))?;
        let size = character.len_utf8();
        if line_comment {
            if character == '\n' {
                line_comment = false;
            }
            index += size;
            continue;
        }
        if block_comment > 0 {
            if index + 1 < bytes.len() && &value[index..index + 2] == "/*" {
                block_comment += 1;
                index += 2;
                continue;
            }
            if index + 1 < bytes.len() && &value[index..index + 2] == "*/" {
                block_comment -= 1;
                index += 2;
                continue;
            }
            index += size;
            continue;
        }
        if let Some(delimiter) = &dollar_quote {
            if value[index..].starts_with(delimiter) {
                index += delimiter.len();
                dollar_quote = None;
            } else {
                index += size;
            }
            continue;
        }
        if let Some(expected) = quote {
            if character == expected {
                if index + size < bytes.len() && bytes[index + size] == expected as u8 {
                    index += size * 2;
                    continue;
                }
                quote = None;
            }
            index += size;
            continue;
        }
        if index + 1 < bytes.len() && &value[index..index + 2] == "--" {
            line_comment = true;
            index += 2;
            continue;
        }
        if index + 1 < bytes.len() && &value[index..index + 2] == "/*" {
            block_comment = 1;
            index += 2;
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
            index += size;
            continue;
        }
        if character == '$'
            && let Some(end) = value[index + 1..].find('$')
        {
            let delimiter = &value[index..=index + end + 1];
            if delimiter[1..delimiter.len() - 1]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                index += delimiter.len();
                dollar_quote = Some(delimiter.to_owned());
                continue;
            }
        }
        if character == ';' {
            statements.push(&value[start..index]);
            start = index + 1;
        }
        index += size;
    }
    if quote.is_some() || dollar_quote.is_some() || block_comment > 0 {
        bail!("unterminated SQL quote or comment");
    }
    if !value[start..].trim().is_empty() {
        statements.push(&value[start..]);
    }
    if statements.is_empty() {
        bail!("SQL must contain a statement");
    }
    Ok(statements
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{SqlRequest, StatementClass, validate_read_only, validate_schema_scope};
    fn request(sql: &str) -> SqlRequest {
        SqlRequest {
            sql: sql.to_string(),
            parameters: Vec::new(),
            row_limit: 100,
            timeout_ms: 10_000,
        }
    }
    #[test]
    fn accepts_documented_read_families() {
        for sql in [
            "SELECT 1",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "EXPLAIN SELECT 1",
            "SHOW server_version",
            "DESCRIBE users",
            "SET TIME ZONE 'UTC'",
        ] {
            assert!(validate_read_only(request(sql)).is_ok(), "{sql}");
        }
    }
    #[test]
    fn rejects_writes_and_multiple_statements() {
        for sql in [
            "INSERT INTO t VALUES (1)",
            "CREATE TABLE t (id INT)",
            "SELECT 1; SELECT 2",
        ] {
            assert!(validate_read_only(request(sql)).is_err(), "{sql}");
        }
    }

    #[test]
    fn rejects_data_modifying_ctes_and_explain_mutations() {
        assert!(
            validate_read_only(request(
                "WITH changed AS (DELETE FROM accounts RETURNING id) SELECT * FROM changed"
            ))
            .is_err()
        );
        assert!(validate_read_only(request(
            "WITH changed AS (UPDATE accounts SET name = 'x' RETURNING id) SELECT * FROM changed"
        ))
        .is_err());
        assert!(validate_read_only(request("EXPLAIN INSERT INTO accounts VALUES (1)")).is_err());
        assert!(
            validate_read_only(request("EXPLAIN ANALYZE DELETE FROM accounts WHERE id = 1"))
                .is_err()
        );
    }
    #[test]
    fn ignores_semicolons_inside_literals_and_comments() {
        let validated =
            validate_read_only(request("SELECT 'a;b' /* ; */ -- ;\n 1")).expect("one statement");
        assert_eq!(validated.statement_class, StatementClass::Read);
    }

    #[test]
    fn handles_escaped_and_dollar_quoted_literals() {
        assert!(validate_read_only(request("SELECT 'a'';b' AS value")).is_ok());
        assert!(validate_read_only(request("SELECT $$ body; still one $$ AS value")).is_ok());
        assert!(validate_read_only(request("SELECT $tag$ body; still one $tag$ AS value")).is_ok());
        assert!(validate_read_only(request("SELECT 1; SELECT 2")).is_err());
    }

    #[test]
    fn schema_scope_rejects_qualified_relations_outside_allowlist() {
        let allowed = vec!["tenant_a".to_string()];
        assert!(validate_schema_scope("SELECT * FROM tenant_a.accounts", &allowed).is_ok());
        assert!(validate_schema_scope("SELECT * FROM tenant_b.accounts", &allowed).is_err());
        assert!(
            validate_schema_scope(
                "SELECT * FROM tenant_a.accounts a JOIN tenant_b.secrets s ON a.id = s.id",
                &allowed
            )
            .is_err()
        );
    }

    #[test]
    fn schema_scope_checks_nested_relations_and_quoted_identifiers() {
        let allowed = vec!["Tenant A".to_string()];
        assert!(
            validate_schema_scope(
                "SELECT a.id FROM (SELECT id FROM \"Tenant A\".accounts) a",
                &allowed
            )
            .is_ok()
        );
        assert!(validate_schema_scope("SELECT id FROM \"Tenant B\".accounts", &allowed).is_err());
    }

    #[test]
    fn schema_scope_fails_closed_when_sql_cannot_be_analyzed() {
        let allowed = vec!["tenant_a".to_string()];
        assert!(validate_schema_scope("SELECT * FROM tenant_a.accounts <=> $1", &allowed).is_err());
        assert!(validate_schema_scope("SELECT * FROM accounts a", &allowed).is_ok());
    }

    #[test]
    fn schema_scope_checks_controlled_write_targets() {
        let allowed = vec!["tenant_a".to_string()];
        assert!(validate_schema_scope("INSERT INTO tenant_a.events VALUES (1)", &allowed).is_ok());
        assert!(validate_schema_scope("UPDATE tenant_a.events SET id = 2", &allowed).is_ok());
        assert!(validate_schema_scope("DELETE FROM tenant_b.events", &allowed).is_err());
    }
}
