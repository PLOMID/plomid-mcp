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
// Authentication policy, bearer-token identity, scope checks, and
// request-scoped tenant/schema context shared by every PLOMID transport.

use std::collections::HashMap;

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AuthFileConfig {
    pub required: Option<bool>,
    pub tokens: Option<Vec<AuthTokenConfig>>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AuthTokenConfig {
    pub token_sha256: String,
    pub subject: String,
    pub tenant: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub schemas: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AuthPolicy {
    pub required: bool,
    tokens: HashMap<String, AuthIdentity>,
}

#[derive(Clone, Debug)]
pub struct AuthIdentity {
    pub subject: String,
    pub tenant: Option<String>,
    pub scopes: Vec<String>,
    pub schemas: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RequestIdentity {
    pub subject: String,
    pub tenant: Option<String>,
    pub scopes: Vec<String>,
    pub schemas: Vec<String>,
}

/// Return the identity attached to the current authenticated request.
pub fn current_identity() -> Option<RequestIdentity> {
    REQUEST_IDENTITY
        .try_with(|identity| identity.clone())
        .ok()
        .flatten()
}

tokio::task_local! {
    static REQUEST_IDENTITY: Option<RequestIdentity>;
}

impl AuthPolicy {
    pub fn verify_token_digest(token: &str, expected_digest: &str) -> bool {
        hex_digest(token.as_bytes()) == expected_digest.trim().to_ascii_lowercase()
    }

    pub fn from_file(config: AuthFileConfig, default_required: bool) -> anyhow::Result<Self> {
        let required = config.required.unwrap_or(default_required);
        let mut tokens = HashMap::new();
        for token in config.tokens.unwrap_or_default() {
            let hash = token.token_sha256.trim().to_ascii_lowercase();
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                anyhow::bail!(
                    "auth token_sha256 must be a 64-character hexadecimal SHA-256 digest"
                );
            }
            if token.subject.trim().is_empty() {
                anyhow::bail!("auth token subject must not be empty");
            }
            if tokens
                .insert(
                    hash,
                    AuthIdentity {
                        subject: token.subject,
                        tenant: token.tenant,
                        scopes: token.scopes,
                        schemas: token.schemas,
                    },
                )
                .is_some()
            {
                anyhow::bail!("duplicate auth token digest");
            }
        }
        if required && tokens.is_empty() {
            anyhow::bail!("HTTP authentication is required but no auth tokens are configured");
        }
        Ok(Self { required, tokens })
    }

    pub fn authenticate(&self, headers: &HeaderMap) -> anyhow::Result<RequestIdentity> {
        let Some(header) = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
        else {
            if self.required {
                anyhow::bail!("AUTH_REQUIRED");
            }
            return Ok(RequestIdentity {
                subject: "local-anonymous".into(),
                tenant: None,
                scopes: vec!["mcp:read".into(), "sql:read".into(), "schema:read".into()],
                schemas: Vec::new(),
            });
        };
        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(|| anyhow::anyhow!("AUTH_REQUIRED"))?;
        let digest = hex_digest(token.as_bytes());
        let identity = self
            .tokens
            .get(&digest)
            .ok_or_else(|| anyhow::anyhow!("ACCESS_DENIED"))?;
        Ok(RequestIdentity {
            subject: identity.subject.clone(),
            tenant: identity.tenant.clone(),
            scopes: identity.scopes.clone(),
            schemas: identity.schemas.clone(),
        })
    }

    pub fn authorize(scope: &str) -> anyhow::Result<()> {
        if let Ok(identity) = REQUEST_IDENTITY.try_with(|identity| identity.clone()) {
            let identity = identity.ok_or_else(|| anyhow::anyhow!("AUTH_REQUIRED"))?;
            if !identity
                .scopes
                .iter()
                .any(|candidate| candidate == scope || candidate == "*")
            {
                anyhow::bail!("ACCESS_DENIED");
            }
        }
        Ok(())
    }

    pub async fn scope<F, T>(&self, identity: RequestIdentity, operation: F) -> T
    where
        F: std::future::Future<Output = T> + Send,
        T: Send,
    {
        REQUEST_IDENTITY.scope(Some(identity), operation).await
    }
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{AuthFileConfig, AuthPolicy, AuthTokenConfig};
    use axum::http::{HeaderMap, HeaderValue};

    const SECRET_DIGEST: &str = "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b";

    #[test]
    fn rejects_required_policy_without_tokens() {
        assert!(
            AuthPolicy::from_file(
                AuthFileConfig {
                    required: Some(true),
                    tokens: None
                },
                false
            )
            .is_err()
        );
    }

    #[test]
    fn authenticates_bearer_identity_from_digest() {
        let policy = AuthPolicy::from_file(
            AuthFileConfig {
                required: Some(true),
                tokens: Some(vec![AuthTokenConfig {
                    token_sha256: SECRET_DIGEST.into(),
                    subject: "agent".into(),
                    tenant: Some("tenant-a".into()),
                    scopes: vec!["sql:read".into()],
                    schemas: vec!["public".into()],
                }]),
            },
            false,
        )
        .expect("policy");
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        let identity = policy.authenticate(&headers).expect("identity");
        assert_eq!(identity.subject, "agent");
        assert_eq!(identity.tenant.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn rejects_missing_and_unknown_bearer_tokens_without_echoing_them() {
        let policy = AuthPolicy::from_file(
            AuthFileConfig {
                required: Some(true),
                tokens: Some(vec![AuthTokenConfig {
                    token_sha256: SECRET_DIGEST.into(),
                    subject: "agent".into(),
                    tenant: None,
                    scopes: vec!["sql:read".into()],
                    schemas: Vec::new(),
                }]),
            },
            false,
        )
        .expect("policy");

        let missing = policy
            .authenticate(&HeaderMap::new())
            .expect_err("missing token");
        assert_eq!(missing.to_string(), "AUTH_REQUIRED");

        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer not-secret"),
        );
        let unknown = policy.authenticate(&headers).expect_err("unknown token");
        assert_eq!(unknown.to_string(), "ACCESS_DENIED");
        assert!(!unknown.to_string().contains("not-secret"));
    }
}
