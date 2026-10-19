# Security Policy

## Reporting a vulnerability

Please do not disclose suspected vulnerabilities in public issues. Contact the
maintainers privately through the organization security channel or the project
maintainer before publishing details. Include a reproduction, affected
version/commit, impact, and a safe contact method.

## Security expectations

- Never expose `PLOMID_DATABASE_URL`, credentials, tokens, or raw authorization
  headers in logs, tool results, issues, or tests.
- Keep HTTP on loopback unless an authenticated and authorized deployment
  boundary is configured.
- Treat documentation and database content as untrusted input.
- Keep read-only database privileges enforced at the PLOMID identity/session
  layer; MCP policy is defense in depth, not the only control.
- Report dependency, supply-chain, protocol, tenant-isolation, and secret
  disclosure issues as security issues.

The project is not production-qualified merely because it compiles; endpoint,
TLS, authentication, authorization, and conformance evidence are release gates.
