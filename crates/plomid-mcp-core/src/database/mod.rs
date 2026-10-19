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
// Database adapter module boundary and public backend contracts.
mod adapter;
mod plomid;
mod unconfigured;

pub use adapter::{
    ColumnInfo, DatabaseHealth, ExecuteRequest, ExecuteResult, IndexInfo, KeyInfo, PlomidDatabase,
    QueryMeta, QueryRequest, QueryResponse, QueryResult, TableDescription, TableInfo,
};
pub use plomid::PlomidSqlDatabase;
pub use unconfigured::UnconfiguredDatabase;
