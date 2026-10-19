#!/usr/bin/env python3
"""Run a real compiled STDIO process against the public MCP contract."""

import json
import os
import subprocess
import sys
from pathlib import Path


binary = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("target/release/plomid-mcp")
database_url = os.environ.get(
    "PLOMID_MCP_SMOKE_DATABASE_URL",
    "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid",
)
process = subprocess.Popen(
    [str(binary), "--database-url", database_url, "--log-level", "debug", "stdio"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1,
)


def exchange(message):
    process.stdin.write(json.dumps(message) + "\n")
    process.stdin.flush()
    return json.loads(process.stdout.readline())


initialized = exchange({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "mcp-client-secret-canary", "version": "1"}}})
assert initialized["result"]["protocolVersion"] == "2025-11-25"
assert "prompts" in initialized["result"]["capabilities"]
process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
process.stdin.flush()
listed = exchange({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
prompts = exchange({"jsonrpc": "2.0", "id": 6, "method": "prompts/list", "params": {}})
assert "plomid.safe-query-workflow" in {prompt["name"] for prompt in prompts["result"]["prompts"]}
prompt = exchange({"jsonrpc": "2.0", "id": 7, "method": "prompts/get", "params": {"name": "plomid.safe-query-workflow"}})
assert "read-only" in prompt["result"]["messages"][0]["content"]["text"]
safe_call = exchange({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "plomid.capabilities", "arguments": {}}})
if os.environ.get("PLOMID_MCP_SMOKE_DATABASE_URL"):
    live_call = exchange({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {"name": "plomid.sql.query", "arguments": {"sql": "SELECT $1::integer AS answer", "parameters": ["42"]}}})
    assert live_call["result"]["structuredContent"]["data"]["rows"] == [[42]]
invalid_call = exchange({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "plomid.sql.query", "arguments": {"sql": "SELECT 1; DROP TABLE synthetic"}}})
unknown_tool = exchange({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "conformance-fixture-tool-not-advertised", "arguments": {}}})
assert "plomid.sql.execute" not in {tool["name"] for tool in listed["result"]["tools"]}
assert any(item["type"] == "text" for item in safe_call["result"]["content"])
assert safe_call["result"]["structuredContent"]["prompts"] == ["plomid.safe-query-workflow"]
assert invalid_call["error"]["code"] == -32602
assert unknown_tool["error"]["code"] == -32602
process.stdin.close()
process.wait(timeout=5)
assert process.returncode == 0
stdout = process.stdout.read()
stderr = process.stderr.read()
assert not stdout.strip(), f"unexpected non-MCP stdout after final response: {stdout!r}"
for forbidden in ("synthetic-secret", "mcp-client-secret-canary", "Bearer", "postgresql://", "DROP TABLE synthetic"):
    assert forbidden.lower() not in stderr.lower()


def negotiate(requested_version):
    child = subprocess.Popen(
        [str(binary), "--database-url", database_url, "stdio"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    child.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": requested_version, "capabilities": {}, "clientInfo": {"name": "protocol-negotiation-smoke", "version": "1"}}}) + "\n")
    child.stdin.flush()
    response = json.loads(child.stdout.readline())
    child.stdin.close()
    child.wait(timeout=5)
    assert child.returncode == 0, child.stderr.read()
    assert not child.stdout.read().strip(), "unexpected non-MCP stdout after initialization"
    return response["result"]["protocolVersion"]


assert negotiate("2025-06-18") == "2025-06-18"
assert negotiate("1900-01-01") == "2025-11-25"

oversized = subprocess.Popen(
    [str(binary), "--database-url", database_url, "stdio"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)
oversized.stdin.write(b"x" * (4 * 1024 * 1024 + 1) + b"\n")
oversized.stdin.close()
oversized.wait(timeout=5)
assert oversized.returncode != 0, "oversized STDIO message did not terminate the session"
assert not oversized.stdout.read(), "oversized STDIO input emitted non-protocol output"
oversized_stderr = oversized.stderr.read()
for forbidden in (b"synthetic-secret", b"postgresql://"):
    assert forbidden not in oversized_stderr
print("STDIO MCP smoke passed: negotiation, tool/prompt discovery, prompt retrieval, safe/invalid/unknown-tool calls, size limit, stdout purity, redaction")
