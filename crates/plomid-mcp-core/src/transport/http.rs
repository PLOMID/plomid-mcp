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
// Streamable HTTP transport with host/origin enforcement,
// authentication middleware, readiness checks, TLS support, and graceful
// local shutdown behavior.

use std::sync::Arc;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::get,
};
use plomid_security::AuthPolicy;
use rmcp::transport::{
    StreamableHttpServerConfig, StreamableHttpService,
    streamable_http_server::session::local::LocalSessionManager,
};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{config::Config, database::PlomidDatabase, mcp::McpServer};

/// Start the production HTTP transport. Remote binds require TLS and
/// configured bearer identities; local binds retain the safe loopback default.
pub async fn serve(server: McpServer, config: &Config) -> Result<()> {
    if !config.bind.ip().is_loopback() && config.tls_cert.is_none() {
        bail!("remote HTTP binding requires configured TLS certificate and key");
    }
    let cancellation = CancellationToken::new();
    let http_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(config.allowed_hosts.clone())
        .with_allowed_origins(config.allowed_origins.clone())
        .enforce_origin_validation()
        .with_json_response(true)
        .with_legacy_session_mode(false)
        .with_max_request_body_bytes(config.max_request_body_bytes)
        .with_cancellation_token(cancellation.clone());
    let database = server.database.clone();
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        http_config,
    );
    let http_state = Arc::new(HttpState {
        auth: config.auth.clone(),
        concurrency: Semaphore::new(config.http_concurrency),
        rate: Mutex::new(HashMap::new()),
        rate_limit_per_minute: config.rate_limit_per_minute,
        metrics: Arc::new(HttpMetrics::default()),
    });
    let app = Router::new()
        .route("/healthz", get(liveness))
        .route("/readyz", get(move || readiness(database.clone())))
        .route("/metrics", get(metrics))
        .nest_service("/mcp", service)
        .layer(from_fn_with_state(http_state.clone(), authenticate))
        .layer(from_fn_with_state(http_state.clone(), observe))
        .with_state(http_state);
    if let (Some(cert), Some(key)) = (&config.tls_cert, &config.tls_key) {
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
        let handle = axum_server::Handle::new();
        let listening_handle = handle.clone();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown(cancellation).await;
            shutdown_handle.graceful_shutdown(Some(Duration::from_secs(30)));
        });
        let server_task = tokio::spawn(
            axum_server::bind_rustls(config.bind, tls)
                .handle(handle)
                .serve(app.into_make_service()),
        );
        if let Some(address) = listening_handle.listening().await {
            let endpoint = format!("https://{address}/mcp");
            tracing::info!(address = %address, "Streamable HTTPS MCP server ready");
            crate::startup::report_ready("Streamable HTTPS", &endpoint);
        }
        server_task.await.context("HTTPS listener task failed")??;
    } else {
        let listener = TcpListener::bind(config.bind).await?;
        let address = listener.local_addr()?;
        let endpoint = format!("http://{address}/mcp");
        tracing::info!(address = %address, "Streamable HTTP MCP server ready");
        crate::startup::report_ready("Streamable HTTP", &endpoint);
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown(cancellation))
            .await?;
    }
    Ok(())
}

struct HttpState {
    auth: AuthPolicy,
    concurrency: Semaphore,
    rate: Mutex<HashMap<String, RateWindow>>,
    rate_limit_per_minute: u32,
    metrics: Arc<HttpMetrics>,
}

#[derive(Default)]
struct HttpMetrics {
    requests_total: AtomicU64,
    responses_4xx_total: AtomicU64,
    responses_5xx_total: AtomicU64,
    active_requests: AtomicU64,
    auth_failures_total: AtomicU64,
    request_duration_ms_total: AtomicU64,
}

struct ActiveRequestGuard {
    metrics: Arc<HttpMetrics>,
    started: Instant,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.metrics.active_requests.fetch_sub(1, Ordering::Relaxed);
        self.metrics
            .request_duration_ms_total
            .fetch_add(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

struct RateWindow {
    started: Instant,
    requests: u32,
}

async fn authenticate(
    State(state): State<Arc<HttpState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !is_mcp_path(request.uri().path()) {
        return next.run(request).await;
    }
    match state.auth.authenticate(request.headers()) {
        Ok(identity) => {
            tracing::info!(
                path = %request.uri().path(),
                "authenticated MCP request"
            );
            if !state.allow_rate(&identity.subject).await {
                return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded\n").into_response();
            }
            let permit = match state.concurrency.try_acquire() {
                Ok(permit) => permit,
                Err(_) => {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        "request concurrency limit exceeded\n",
                    )
                        .into_response();
                }
            };
            let response = state.auth.scope(identity, next.run(request)).await;
            drop(permit);
            response
        }
        Err(error) if error.to_string() == "AUTH_REQUIRED" => {
            state
                .metrics
                .auth_failures_total
                .fetch_add(1, Ordering::Relaxed);
            (StatusCode::UNAUTHORIZED, "authentication required\n").into_response()
        }
        Err(_) => {
            state
                .metrics
                .auth_failures_total
                .fetch_add(1, Ordering::Relaxed);
            (StatusCode::FORBIDDEN, "access denied\n").into_response()
        }
    }
}

fn is_mcp_path(path: &str) -> bool {
    path == "/mcp" || path.starts_with("/mcp/")
}

impl HttpState {
    async fn allow_rate(&self, subject: &str) -> bool {
        let now = Instant::now();
        let mut windows = self.rate.lock().await;
        windows.retain(|_, window| now.duration_since(window.started) < Duration::from_secs(60));
        let window = windows.entry(subject.to_owned()).or_insert(RateWindow {
            started: now,
            requests: 0,
        });
        if now.duration_since(window.started) >= Duration::from_secs(60) {
            window.started = now;
            window.requests = 0;
        }
        if window.requests >= self.rate_limit_per_minute {
            return false;
        }
        window.requests += 1;
        true
    }
}

async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

async fn readiness(database: Arc<dyn PlomidDatabase>) -> impl IntoResponse {
    // Do not expose connection details through the readiness endpoint.
    let ready = tokio::time::timeout(Duration::from_secs(2), database.health())
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|health| health.connected);
    if ready {
        (StatusCode::OK, "ready\n")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "database unavailable\n")
    }
}

async fn observe(
    State(state): State<Arc<HttpState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let request_id = format!(
        "http-{}",
        state.metrics.requests_total.fetch_add(1, Ordering::Relaxed) + 1
    );
    let started = Instant::now();
    state
        .metrics
        .active_requests
        .fetch_add(1, Ordering::Relaxed);
    let _active_request = ActiveRequestGuard {
        metrics: state.metrics.clone(),
        started,
    };
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path()
    );
    let response = next.run(request).instrument(span).await;
    if response.status().is_client_error() {
        state
            .metrics
            .responses_4xx_total
            .fetch_add(1, Ordering::Relaxed);
    } else if response.status().is_server_error() {
        state
            .metrics
            .responses_5xx_total
            .fetch_add(1, Ordering::Relaxed);
    }
    response
}

async fn metrics(State(state): State<Arc<HttpState>>) -> impl IntoResponse {
    let m = &state.metrics;
    let body = format!(
        "# TYPE plomid_http_requests_total counter\nplomid_http_requests_total {}\n# TYPE plomid_http_responses_4xx_total counter\nplomid_http_responses_4xx_total {}\n# TYPE plomid_http_responses_5xx_total counter\nplomid_http_responses_5xx_total {}\n# TYPE plomid_http_active_requests gauge\nplomid_http_active_requests {}\n# TYPE plomid_http_auth_failures_total counter\nplomid_http_auth_failures_total {}\n# TYPE plomid_http_request_duration_ms_total counter\nplomid_http_request_duration_ms_total {}\n",
        m.requests_total.load(Ordering::Relaxed),
        m.responses_4xx_total.load(Ordering::Relaxed),
        m.responses_5xx_total.load(Ordering::Relaxed),
        m.active_requests.load(Ordering::Relaxed),
        m.auth_failures_total.load(Ordering::Relaxed),
        m.request_duration_ms_total.load(Ordering::Relaxed),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

async fn shutdown(cancellation: CancellationToken) {
    if tokio::signal::ctrl_c().await.is_ok() {
        tracing::info!("shutdown signal received");
        cancellation.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveRequestGuard, HttpMetrics, is_mcp_path};
    use std::sync::{Arc, atomic::Ordering};
    use std::time::Instant;

    #[test]
    fn active_request_metrics_are_released_when_guard_is_dropped() {
        let metrics = Arc::new(HttpMetrics::default());
        metrics.active_requests.fetch_add(1, Ordering::Relaxed);
        let guard = ActiveRequestGuard {
            metrics: metrics.clone(),
            started: Instant::now(),
        };
        drop(guard);
        assert_eq!(metrics.active_requests.load(Ordering::Relaxed), 0);
        assert!(metrics.request_duration_ms_total.load(Ordering::Relaxed) < 1000);
    }

    #[test]
    fn mcp_path_matching_does_not_treat_similar_paths_as_mcp() {
        assert!(is_mcp_path("/mcp"));
        assert!(is_mcp_path("/mcp/"));
        assert!(is_mcp_path("/mcp/session"));
        assert!(!is_mcp_path("/mcp-invalid"));
        assert!(!is_mcp_path("/mcpx"));
    }
}
