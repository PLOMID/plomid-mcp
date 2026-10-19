# Contributing to PLOMID-MCP

PLOMID-MCP is an Apache-2.0 licensed open-source project. Contributions must
preserve the documented MCP contract, security boundaries, and PLOMID adapter
invariants.

## Development

Requirements are pinned in `rust-toolchain.toml`. Before opening a change, run:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Do not add credentials, customer data, generated secrets, or private PLOMID
endpoint details to commits. New tools require a documented contract, bounded
inputs/outputs, authorization classification, negative tests, and an explicit
capability status.

## Pull requests

Describe the behavior change, security impact, compatibility assumptions, and
verification evidence. Changes to public tool names, schemas, access levels,
transport behavior, or error codes require updates to `docs/tool-contract.md`
and `PLAN.md`.
