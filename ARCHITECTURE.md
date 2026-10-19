# PLOMID-MCP Architecture

## Scope

PLOMID-MCP is a standalone MCP adapter and policy boundary. It does not read
PLOMID storage files, link to private engine internals, provide shell/file
execution, or create a second database.

## Current architecture

```text
MCP client
    |                         |
    | STDIO / rmcp             | Streamable HTTP / rmcp
    v                         v
PLOMID-MCP application and transport boundary
    |-- CLI/configuration and redaction
    |-- transport::stdio / transport::http
    |-- MCP tool contracts and policy
    |-- status/capability services
    `-- Arc<dyn PlomidDatabase>
            |
            `-- PLOMID PostgreSQL-wire driver with pooling and policy controls
```

The local HTTP transport is enabled only through the explicit `http` command.
Loopback HTTP is the default. Remote deployment requires direct TLS and
configured bearer identities; authenticated requests receive scope, tenant,
and schema policy before reaching MCP tools.

## Boundaries and invariants

1. All database access must cross `PlomidDatabase`.
2. STDOUT is reserved for MCP protocol data in STDIO mode; logs use STDERR.
3. Database URLs and credentials are never returned by tools or logs.
4. Write operations are advertised only in explicitly enabled controlled-write
   mode and require approval, mutation validation, and idempotency protection.
5. Every tool response is bounded by server-side limits.
6. Capability claims distinguish configured, implemented, and integration-validated behavior.
7. HTTP routes are explicit: `/mcp` for MCP, `/healthz` for liveness, and
    `/readyz` for readiness; no catch-all database endpoint is exposed.
8. Non-loopback HTTP binds require TLS and configured authentication policy.

## Dependency decisions

- MCP: official published `rmcp` 3.5.1, pinned through `Cargo.lock`.
- Runtime: Tokio.
- HTTP: Axum hosting the official RMCP Streamable HTTP service.
- Configuration: Clap CLI and environment variables, with CLI precedence.
- Serialization: Serde/Serde JSON.
- Documentation hashing: SHA-256 for provenance markers.
- Database: adapter trait first; concrete driver selected only after PLOMID compatibility testing.
