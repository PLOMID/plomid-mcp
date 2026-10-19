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
// SQL policy module boundary and public request-validation API.
mod contracts;
mod policy;

pub use contracts::{
    DEFAULT_ROW_LIMIT, DEFAULT_TIMEOUT_MS, ExplainRequest, MAX_PARAMETER_BYTES, MAX_PARAMETERS,
    MAX_ROW_LIMIT, MAX_SQL_BYTES, MAX_TIMEOUT_MS, SqlRequest, StatementClass, ValidatedSql,
    ValidatedWrite, WriteRequest,
};
pub use policy::{validate_explain, validate_read_only, validate_schema_scope, validate_write};
