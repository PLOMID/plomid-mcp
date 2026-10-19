<div align="center">

<img src="assets/readme/plomid-mcp-banner.svg" alt="PLOMID-MCP connects MCP clients through a controlled integration layer to an independent PLOMID database" width="100%">

**Connect AI clients and agents to PLOMID through the Model Context Protocol.**

Rust · STDIO · Streamable HTTP · supported PLOMID SQL protocol

</div>

---

PLOMID-MCP is the Rust **Model Context Protocol integration layer** for PLOMID.
MCP clients connect over STDIO or Streamable HTTP to discover and invoke
database status, capabilities, SQL query/explain, schema inspection, and—when
configured—controlled SQL execution. The server applies operation-specific
authorization and limits, then communicates with an independently running
PLOMID database through its supported SQL protocol. It neither bundles nor
replaces the database.

**PLOMID** is the database and data infrastructure platform. **PLOMID-MCP**
connects it to **MCP-compatible clients and agents** through a consistent tool
interface. The default mode supports inspection and queries; controlled DML is
also available through a separate, approval-gated mode.

## On this page

[Capabilities](#capabilities) · [Architecture](#architecture) ·
[Request lifecycle](#request-lifecycle) · [Quick start](#quick-start) ·
[Tool reference](#tool-reference) · [Configuration](#configuration) ·
[Security](#security-model) · [Client integration](#client-integration) ·
[Deployment](#deployment) · [Release targets](#release-targets) ·
[Testing](#testing-and-verification) ·
[Troubleshooting](#troubleshooting) · [Development](#development-and-contributing)

## Capabilities

| Area | What this server provides |
|---|---|
| MCP discovery | Initialization, tool schemas, and `plomid.safe-query-workflow`. |
| SQL | Parameterized read queries and `EXPLAIN`, constrained by SQL policy and execution bounds. |
| Schema inspection | Visible databases, schemas, tables, and table metadata. |
| SQL execution | Query, explain, and controlled DML via `plomid.sql.execute`; DML is disabled until explicitly configured and approved. |
| Transports | STDIO for a client-managed local process; Streamable HTTP for local or networked clients. |
| Deployment security | HTTP bearer identities/scopes, Origin/Host checks, TLS, database certificate verification, redacted errors, and bounded resources. |

## Architecture

The server is a protocol adapter, not a database replacement. Tools and prompts
are served by the MCP process; SQL and database authorization remain at the
database connection boundary.

<p align="center"><img src="assets/readme/architecture.svg" alt="Three trust boundaries: MCP consumer, PLOMID-MCP integration service, and independent PLOMID database service" width="100%"></p>

Tool contracts, authorization, SQL validation, and resource bounds apply
according to the operation. The adapter then uses PLOMID's supported SQL
protocol. Database grants remain authoritative even when application-level
controls are configured. Animated copper signals are progressive enhancement;
the static arrows and signal markers retain the full request path.

## Request lifecycle

Status/capabilities report state; metadata tools inspect visible objects; SQL
tools are parsed and bounded; the write tool has additional approval checks.
These paths intentionally do not share identical policy.

<p align="center"><img src="assets/readme/request-lifecycle.svg" alt="A tool call is dispatched and validated, receives applicable policy checks, executes over PLOMID SQL, and returns bounded structured content or a stable error" width="100%"></p>

HTTP bearer authentication and scope authorization apply to MCP requests.
STDIO has no separate HTTP bearer layer; local process access and database
credentials define that boundary. Query results preserve columns and rows
within row and encoded-byte caps. Backend failures become stable, redacted
errors.

## Quick start

### Requirements

- A running PLOMID server reachable over its supported PostgreSQL-wire SQL endpoint.
- The Rust toolchain specified by [`rust-toolchain.toml`](rust-toolchain.toml), to build from source.
- An MCP-compatible client for STDIO or Streamable HTTP.

### Build and run locally

```bash
export PLOMID_DATABASE_URL='postgresql://USER:PASSWORD@127.0.0.1:5432/DATABASE'
cargo build --release --locked
./target/release/plomid-mcp validate
./target/release/plomid-mcp stdio
```

Replace the URL placeholders with your host, database, and credentials. Use a
protected environment or secret store for real credentials. `validate` checks
configuration; `doctor` checks live connectivity. STDIO protocol messages use
stdout and diagnostics use stderr. For local PLOMID, follow its supported
[quickstart](https://github.com/plomid/plomid/blob/main/docs/v0.1.0/getting-started/quickstart.mdx)
and use an isolated test data directory.

### First query and schema discovery

After initialization, discover tools with this MCP request:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
```

Then invoke `plomid.sql.query` with these tool arguments:

```json
{
  "sql": "SELECT $1::integer AS answer",
  "parameters": ["42"],
  "row_limit": 10,
  "timeout_ms": 5000
}
```

Structured content includes `data.columns`, `data.rows`, and `data.truncated`;
metadata includes request timing and warnings. Empty results preserve column
metadata and return an empty `rows` array. Defaults are 100 rows and 10 seconds;
tool requests clamp to 1–10,000 rows and 1–120,000 ms, then process-wide caps
apply. Discover objects using `plomid.schema.list_schemas`,
`plomid.schema.list_tables` with `{"schema":"public"}`, and
`plomid.schema.describe_table` with schema/table strings. Visibility depends on
database permissions and HTTP identity schema restrictions.

## Tool reference

`tools/list` returns eight tools in the standard configuration; the ninth,
`plomid.sql.execute`, is added when controlled writes are enabled. Scope names
apply to authenticated HTTP identities; in STDIO mode there is no bearer
identity layer. All structured outputs are MCP `structuredContent`. The status `ok`
field is part of the response schema; use `database_status` (`connected`,
`configured-but-not-connected`, or `not-configured`) to determine connectivity.

| Public name | Purpose · inputs | Result | Access |
|---|---|---|---|
| `plomid.status` | Report server mode and database status. No inputs. | `schema_version`, `ok`, `mode`, `database_configured`, `database_status`, `warnings`. | `mcp:read`; status check. |
| `plomid.capabilities` | Describe advertised tools, prompt, feature domains, mode, and limits. No inputs. | `schema_version`, `server_mode`, `database_configured`, `database_integration`, `tools`, `prompts`, `limits`, `feature_domains`. | `mcp:read`; descriptive. |
| `plomid.sql.query` | Execute one permitted query. `sql`; optional `parameters: string[]` (default `[]`), `row_limit` (100), `timeout_ms` (10,000). | `schema_version`, `data.columns`, `data.rows`, `data.truncated`, `data.warnings`; `meta.request_id`, `elapsed_ms`, `truncated`, `warnings`, `retryable`. | `sql:read`; query policy and bounds. |
| `plomid.sql.explain` | Inspect a plan. `sql` must be `EXPLAIN`; optional `parameters`, `row_limit` (100), `timeout_ms` (10,000). | Same bounded query result envelope; plan rows depend on PLOMID's supported format. | `sql:read`; `EXPLAIN ANALYZE` executes its inner statement. |
| `plomid.schema.list_databases` | List databases visible to the connected PLOMID identity. No inputs. | Array of database names. | `schema:read`; database visibility applies. |
| `plomid.schema.list_schemas` | List schemas visible to the connected identity. No inputs. | Array of schema names, filtered by authenticated HTTP schema allowlist. | `schema:read`. |
| `plomid.schema.list_tables` | List tables. Optional `schema` string or null (omitted/null defaults to `public`). | Array of `{schema, name, kind}` objects. | `schema:read`; requested schema must be allowed. |
| `plomid.schema.describe_table` | Describe a table. Required `schema: string`, `table: string`. | `{schema, name, columns, keys, indexes}`; columns include name/type/nullability/position. | `schema:read`; schema and database visibility apply. |
| `plomid.sql.execute` | Execute one controlled DML statement. `sql`; optional `parameters: string[]`, `timeout_ms` (10,000); required `approval_token`, `idempotency_key`. | `schema_version`, `data: {affected_rows, committed}`, and top-level `committed`. | Controlled-write mode and `write_enabled`; HTTP identity needs `sql:write`; valid approval token, unique key, permitted DML, database grants. |

### Prompt (not a tool)

`plomid.safe-query-workflow` is the sole MCP prompt. It guides clients to
inspect capabilities and authorized schemas, parameterize values, keep
ordinary query work to one read statement, and respect limits. It explicitly
allows a write only when an authorized write tool is advertised; enforcement
remains in server code and database grants. Discover it with `prompts/list`,
then retrieve it using `prompts/get` with `{"name":"plomid.safe-query-workflow"}`;
it takes no prompt arguments.

## Example workflows

### Discover tools and inspect database state

After MCP initialization, call `plomid.status` and `plomid.capabilities`. A
complete status call uses the MCP `tools/call` method:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"plomid.status","arguments":{}}}
```

For metadata discovery, call each tool individually with the listed arguments:

| Tool | Arguments |
|---|---|
| `plomid.schema.list_databases` | `{}` |
| `plomid.schema.list_schemas` | `{}` |
| `plomid.schema.list_tables` | `{"schema":"public"}` |
| `plomid.schema.describe_table` | `{"schema":"public","table":"YOUR_TABLE"}` |

### Query and explain

Use `plomid.sql.query` with the parameterized example in [Quick start](#first-query-and-schema-discovery).
To inspect a plan, call `plomid.sql.explain`:

```json
{"sql":"EXPLAIN SELECT * FROM YOUR_TABLE WHERE id = $1","parameters":["42"],"row_limit":20,"timeout_ms":5000}
```

Replace `YOUR_TABLE` and `id` with names from schema discovery.

An empty query result has the selected `columns`, `rows: []`, and
`truncated: false`. If rows or encoded output exceed configured caps, the
result is marked truncated; lower `row_limit` or select fewer columns for a
smaller response.

### Controlled write

`plomid.sql.execute` is a first-class, separately gated operation. To advertise
it, configure all of the following on the server:

```toml
mode = "controlled-write"
write_enabled = true
write_approval_token_sha256 = "SHA256_DIGEST_OF_PRIVATE_APPROVAL_TOKEN"
write_approval_ttl_secs = 300
```

Generate and distribute the raw approval token through a secret manager; set
only its SHA-256 digest in this configuration. For authenticated HTTP, the
calling identity must additionally have the `sql:write` scope. In all
transports, the caller supplies that raw approval token per operation, a
16–256 byte `idempotency_key`, one permitted DML statement, and parameters
as needed. The connected database role must have the relevant write grants.
The key is an in-process reservation for the configured TTL, not a durable
cross-restart deduplication store. For authenticated HTTP, add `sql:write` to
the bearer identity's configured scopes. The HTTP bearer token and per-call
write approval token are separate credentials; do not reuse one for the other.
Keep the raw approval token in the invoking client's secret store and configure
the server with only its SHA-256 digest. The HTTP token's digest belongs in
`auth.tokens[].token_sha256`; that identity needs `sql:write` in its scopes.

For authenticated HTTP, the relevant identity entry therefore includes the
write scope and the digest of a separate bearer token:

```toml
[auth]
required = true

[[auth.tokens]]
token_sha256 = "SHA256_DIGEST_OF_HTTP_BEARER_TOKEN"
subject = "approved-writer"
scopes = ["mcp:read", "sql:read", "schema:read", "sql:write"]
schemas = ["public"]
```

For STDIO, omit the HTTP bearer identity block; local process access still
does not bypass the controlled-write mode, approval token, DML policy, or
database grants.

Example tool arguments against an authorized disposable database (the table
and row must exist; names and values below are illustrative placeholders):

```json
{
  "sql": "UPDATE write_probe SET note = $1 WHERE probe_id = $2",
  "parameters": ["approved value", "probe-row-01"],
  "timeout_ms": 5000,
  "approval_token": "INJECT_FROM_APPROVAL_SECRET_STORE",
  "idempotency_key": "change-ticket-2026-000041"
}
```

On success, the result reports `data.affected_rows` and `data.committed`. Only
one DML statement is accepted; administrative, transaction, and multi-statement
SQL are not part of this workflow. A timeout during commit can leave the
outcome uncertain—check database state before retrying with a new key. See the
[write-tool contract](mcp-docs/tools/write-contract.mdx) for the same safeguards.

## Configuration

Settings come from TOML, environment variables, or CLI flags; explicit CLI
values override TOML. `--config` / `PLOMID_MCP_CONFIG` selects a file.

| TOML key | Environment / CLI | Default | Notes |
|---|---|---|---|
| — | `PLOMID_MCP_CONFIG` / `--config` | Unset | Select the TOML configuration file. |
| `database_url` | `PLOMID_DATABASE_URL` / `--database-url` | Unset | Required in read-only mode; PostgreSQL URL. |
| `mode` | `PLOMID_MCP_MODE` / `--mode` | `read-only` | `read-only` or `controlled-write`. |
| `bind` | `PLOMID_MCP_BIND` / `--bind` | `127.0.0.1:8080` | Non-loopback requires direct TLS. |
| `allowed_hosts` | `PLOMID_MCP_ALLOWED_HOSTS` / `--allowed-hosts` | `localhost,127.0.0.1,::1` | HTTP Host allowlist. |
| `allowed_origins` | `PLOMID_MCP_ALLOWED_ORIGINS` / `--allowed-origins` | `http://localhost:*`, `http://127.0.0.1:*`, `http://[::1]:*` | Browser Origin allowlist. |
| `auth.required` | TOML only | False loopback; true remote | Remote default requires token identities; explicitly setting false disables that protection. |
| `auth.tokens[]` | TOML only | None | `token_sha256`, required `subject`, optional `tenant`, `scopes` (default `[]`), `schemas` (default `[]`). Never store raw token. |
| `max_request_body_bytes` | `PLOMID_MCP_MAX_BODY_BYTES` / `--max-request-body-bytes` | 4 MiB | Valid 1 byte–64 MiB. |
| `max_rows` | `PLOMID_MCP_MAX_ROWS` / `--max-rows` | 1,000 | Valid 1–100,000; SQL tool maximum 10,000. |
| `max_result_bytes` | `PLOMID_MCP_MAX_RESULT_BYTES` / `--max-result-bytes` | 8 MiB | Valid 1 byte–256 MiB. |
| `pool_size` | `PLOMID_MCP_POOL_SIZE` / `--pool-size` | 16 | Valid 1–256. |
| `max_concurrent_queries` | `PLOMID_MCP_MAX_CONCURRENT_QUERIES` / `--max-concurrent-queries` | 16 | Valid 1–256. |
| `http_concurrency` | `PLOMID_MCP_HTTP_CONCURRENCY` / `--http-concurrency` | 64 | Valid 1–4,096. |
| `rate_limit_per_minute` | `PLOMID_MCP_RATE_LIMIT_PER_MINUTE` / `--rate-limit-per-minute` | 600 | Valid 1–1,000,000 requests per principal/minute. |
| `write_enabled` | `PLOMID_MCP_WRITE_ENABLED` / `--write-enabled` | `false` | Requires controlled-write mode and approval digest. |
| `write_approval_token_sha256` | `PLOMID_MCP_WRITE_APPROVAL_TOKEN_SHA256` / `--write-approval-token-sha256` | Unset | SHA-256 digest; never raw token. |
| `write_approval_ttl_secs` | `PLOMID_MCP_WRITE_APPROVAL_TTL_SECS` / `--write-approval-ttl-secs` | 300 | Idempotency-key reservation lifetime. |
| `tls_cert`, `tls_key` | `PLOMID_MCP_TLS_CERT`, `PLOMID_MCP_TLS_KEY` / `--tls-cert`, `--tls-key` | Unset | Configure together for HTTPS. |
| `log_level` | `RUST_LOG` / `--log-level` | `info` | Diagnostics go to stderr: colorized human-readable output in a terminal, JSON when redirected. Set `NO_COLOR` to disable terminal color. |

SQL tool limits also include 256 KiB SQL, 64 parameters up to 64 KiB each, and
120 seconds maximum timeout. See [example TOML](plomid-mcp.example.toml) and
the [configuration reference](mcp-docs/configuration/reference.mdx).

Local HTTP startup:

```bash
PLOMID_DATABASE_URL='postgresql://USER:PASSWORD@127.0.0.1:5432/DATABASE' \
  ./target/release/plomid-mcp http --bind 127.0.0.1:8080
```

The endpoint is `http://127.0.0.1:8080/mcp`. Loopback HTTP is for local
development; do not expose it remotely without TLS and authentication.

## Security model

Default query mode supports inspection and querying without hiding the
implemented write path. Controlled writes are separately enabled and require
approval, identity authorization, and database privileges.

<p align="center"><img src="assets/readme/security-model.svg" alt="Default SQL and resource controls, configured HTTP identity and TLS, and a separate approval-gated controlled-write path" width="100%"></p>

| Boundary | Behavior |
|---|---|
| Query policy | Parser-backed classification enforces one statement and blocks mutations, administration, transactions, and data-modifying CTEs from read tools. |
| Parameters | Bounded values use protocol binding where supported; lexical-aware compatibility substitution is reported in warnings. |
| Limits | SQL/parameter size, execution time, rows, result bytes, pool, and concurrency are bounded. |
| HTTP identity | Bearer tokens are stored as SHA-256 digests; scopes authorize tool classes and schema lists can limit visibility. Database grants remain decisive. |
| TLS | Database chain and hostname are verified. Remote HTTP requires a server certificate/key. No insecure verification fallback exists. |
| Redaction | Credentials, raw backend errors, and client-controlled peer identity are not exposed in MCP results or ordinary logs. |
| Controlled writes | Off by default: requires controlled-write mode, `write_enabled`, approval digest/token, 16–256 byte idempotency key, permitted DML, authenticated HTTP `sql:write` scope, and database write grants. |

Prompts are not authorization. Use least-privilege database grants and an
authorized disposable database when qualifying writes.

## Client integration

### STDIO

Launch the binary directly and inject credentials via the MCP client's
protected environment/secret support:

```json
{
  "mcpServers": {
    "plomid": {
      "command": "/absolute/path/to/plomid-mcp/target/release/plomid-mcp",
      "args": ["stdio"],
      "env": {"PLOMID_DATABASE_URL": "postgresql://USER:PASSWORD@127.0.0.1:5432/DATABASE"}
    }
  }
}
```

Replace the path and URL placeholders. Keep stdout free of banners and wrappers.

### Streamable HTTP

Local clients use `http://127.0.0.1:8080/mcp`. Remote deployment requires
HTTPS, allowed Host/Origin values, bearer identities, and a matching server
certificate. Raw tokens belong in the client secret store, not server config.

```toml
bind = "0.0.0.0:8443"
allowed_hosts = ["mcp.example.internal"]
allowed_origins = ["https://client.example.internal"]
tls_cert = "/etc/plomid-mcp/tls/server.crt"
tls_key = "/etc/plomid-mcp/tls/server.key"
[auth]
required = true
[[auth.tokens]]
token_sha256 = "REPLACE_WITH_64_HEX_CHARACTER_SHA256_DIGEST"
subject = "data-assistant"
scopes = ["mcp:read", "sql:read", "schema:read"]
schemas = ["public"]
```

Database CA configuration is separate. Set `SSL_CERT_FILE` for a private CA
and preserve hostname verification.

## Deployment

STDIO is client-launched; Streamable HTTP is a listening service. Either can
run directly or in a container. Every mode connects to an independently
deployed PLOMID database.

<p align="center"><img src="assets/readme/deployment.svg" alt="Local STDIO or Streamable HTTP deployment, optionally containerized, connecting to an independent PLOMID database" width="100%"></p>

### Docker

The multi-stage Dockerfile builds the workspace and copies the binary into a
Debian slim runtime running as non-root UID/GID 10001. The image does not
contain PLOMID, credentials, or database files.

```bash
docker build -t plomid-mcp:local .
docker run --rm -i \
  -e PLOMID_DATABASE_URL='postgresql://USER:PASSWORD@host.docker.internal:5432/DATABASE' \
  plomid-mcp:local stdio
```

Docker Desktop's `host.docker.internal` usually reaches a host database. On
Linux, use a shared network/service name or configured host-gateway. Container
`localhost` means the container. For HTTP, mount TOML and TLS files read-only,
set `PLOMID_MCP_CONFIG`, configure auth and Host/Origin policy, and launch
`http`. Set `SSL_CERT_FILE` for private database trust roots. Inject secrets at
runtime, never into the image.

`/healthz` is process liveness; `/readyz` checks database connectivity and
returns 503 when unavailable; `/metrics` exposes process counters. Bearer auth
protects `/mcp`, not health or metrics: restrict those endpoints at the network
boundary. `docker stop` requests graceful shutdown.

## Release targets

The tag-triggered release workflow is configured to build native binaries and platform packages
for Linux x86-64 and ARM64, macOS Intel and Apple silicon, and Windows x86-64
and ARM64. Linux/macOS packages use `.tar.gz`; Windows packages use `.zip`.
Each package includes the executable, example configuration, license notices,
CycloneDX SBOM, and a SHA-256 checksum. Windows package qualification runs the
STDIO protocol smoke; Unix-like targets also run the full transport contract.

These are configured build targets, not a claim that every target has already
passed on hosted runners. No release is published by this README or local build;
the release workflow runs only when its version-tag trigger is pushed.

## Testing and verification

Workspace tests cover unit, configuration, and security behavior; live tests
are opt-in. Recorded local qualification for this implementation includes:

- Live PLOMID parameterized queries, empty-result metadata, bounded results,
  invalid SQL, and schema discovery.
- MCP query operations over STDIO and Streamable HTTP against live PLOMID.
- Containerized STDIO discovery and parameterized query over trusted database
  TLS; authenticated HTTPS, unauthenticated rejection, valid MCP calls, and
  readiness checks.
- Controlled-write safeguards and bounded write behavior against an authorized
  disposable test database, never production data.
- Formatting, locked workspace tests, Clippy with warnings denied, release
  build, package/docs validation, transport smoke tests, and `git diff --check`.

Re-run local checks:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --workspace --release --locked
python3 scripts/mcp_stdio_smoke.py target/release/plomid-mcp
python3 scripts/mcp_http_smoke.py target/release/plomid-mcp
python3 scripts/validate_docs.py
```

[`tests/plomid_live.rs`](tests/plomid_live.rs) has ignored live tests requiring
an explicitly authorized `PLOMID_TEST_DATABASE_URL`; the TLS case additionally
needs `PLOMID_TEST_TLS_DATABASE_URL` and verified `SSL_CERT_FILE`. The
[`scripts/live_plomid_qualification.py`](scripts/live_plomid_qualification.py)
harness accepts an authorized MCP HTTP endpoint via `--url` or
`PLOMID_MCP_QUALIFICATION_URL`; it checks query/error behavior, concurrency,
timeout and cancellation recovery. These do not run in default workspace tests
or automatically in every CI job.

## Troubleshooting

| Symptom | Check |
|---|---|
| Database URL required | Set `PLOMID_DATABASE_URL` or TOML `database_url`. |
| `doctor` or `/readyz` fails | Check PLOMID, host/port/database, credentials, and container routing. |
| TLS chain/name failure | Trust the CA with `SSL_CERT_FILE`; connect with a name in the certificate SAN. Never disable verification. |
| HTTP 401 | Supply a configured bearer token. |
| HTTP 403 | Check token digest, scope, and schema allowlist. |
| `BACKEND_UNAVAILABLE` | Inspect protected diagnostics; raw backend details are hidden intentionally. |
| SQL policy rejection | Check mode, statement class, schema scope, parameters, and documented limits. |
| Container cannot reach database | Use a reachable host address or shared-network service name, not container localhost. |
| STDIO protocol error | Launch `plomid-mcp stdio` directly; keep stdout protocol-only. |

## Development and contributing

The workspace contains the root launcher, reusable
[`plomid-mcp-core`](crates/plomid-mcp-core/src/lib.rs), and
[`plomid-security`](crates/plomid-security/src/lib.rs). See
[tool registration](crates/plomid-mcp-core/src/mcp/tools.rs),
[SQL policy](crates/plomid-mcp-core/src/sql/policy.rs),
[database adapter](crates/plomid-mcp-core/src/database/plomid.rs), and
[HTTP transport](crates/plomid-mcp-core/src/transport/http.rs).

Preserve tool contracts, SQL safety, execution bounds, transport behavior, and
credential redaction. Read [`CONTRIBUTING.md`](CONTRIBUTING.md), the
[tool catalog](mcp-docs/tools/catalog.mdx),
[write-tool contract](mcp-docs/tools/write-contract.mdx), and
[security reporting guidance](SECURITY.md).

Licensed under [Apache License 2.0](LICENSE). For database installation and
behavior, see the [PLOMID repository](https://github.com/plomid/plomid). For
this MCP server, use the [repository issue tracker](https://github.com/plomid/plomid-mcp/issues).

---

<div align="center">
  <sub>Part of the <a href="https://github.com/plomid/plomid">PLOMID ecosystem</a> · Developed by <a href="https://github.com/sainathsapa">Sainath Sapa</a></sub>
</div>

### Cite this project

```bibtex
@software{sapa_plomid_mcp,
  author  = {Sapa, Sainath},
  title   = {{PLOMID-MCP}: Connect AI Clients and Agents to {PLOMID} through the {Model Context Protocol}},
  url     = {https://github.com/plomid/plomid-mcp},
  version = {0.1.0},
  license = {Apache-2.0}
}
```
