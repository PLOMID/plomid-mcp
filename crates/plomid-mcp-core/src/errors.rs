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
// Stable internal error categories and safe model-facing error
// translation shared across the MCP server boundary.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PublicError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl PublicError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_INPUT",
            message: message.into(),
            retryable: false,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "NOT_FOUND",
            message: message.into(),
            retryable: false,
        }
    }
}
