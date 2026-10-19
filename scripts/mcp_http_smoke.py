#!/usr/bin/env python3
"""Exercise the compiled Streamable HTTP server on loopback."""

import json
import os
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


binary = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("target/release/plomid-mcp")
database_url = os.environ.get(
    "PLOMID_MCP_SMOKE_DATABASE_URL",
    "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid",
)
with socket.socket() as probe:
    probe.bind(("127.0.0.1", 0))
    port = probe.getsockname()[1]
base = f"http://127.0.0.1:{port}"
process = subprocess.Popen(
    [str(binary), "--database-url", database_url, "--bind", f"127.0.0.1:{port}", "--max-request-body-bytes", "1024", "http"],
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
)


def get(path):
    with urllib.request.urlopen(base + path, timeout=2) as response:
        return response.status, response.read().decode()


try:
    deadline = time.monotonic() + 15
    while True:
        if process.poll() is not None:
            raise RuntimeError(f"server exited during startup ({process.returncode})")
        try:
            live_status, _ = get("/healthz")
            break
        except (OSError, urllib.error.URLError):
            if time.monotonic() >= deadline:
                raise RuntimeError("server readiness deadline exceeded")
            time.sleep(0.1)
    assert live_status == 200
    if os.environ.get("PLOMID_MCP_SMOKE_DATABASE_URL"):
        ready_status, ready_body = get("/readyz")
        assert ready_status == 200 and ready_body == "ready\n"
    else:
        try:
            get("/readyz")
            raise AssertionError("database-unavailable readiness unexpectedly succeeded")
        except urllib.error.HTTPError as error:
            assert error.code == 503

    def rpc(request_id, method, params):
        payload = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}).encode()
        request = urllib.request.Request(
            base + "/mcp",
            data=payload,
            headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
        )
        with urllib.request.urlopen(request, timeout=10) as response:
            text = response.read().decode()
            if response.headers.get_content_type() == "text/event-stream":
                text = next(line[6:] for line in text.splitlines() if line.startswith("data: "))
            return json.loads(text)

    initialized = rpc(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "mcp-client-secret-canary", "version": "1"}})
    assert initialized["result"]["protocolVersion"] == "2025-11-25"
    assert "prompts" in initialized["result"]["capabilities"]
    notification = urllib.request.Request(
        base + "/mcp",
        data=json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
    )
    with urllib.request.urlopen(notification, timeout=10) as response:
        assert response.status in (200, 202, 204)
    oversized = urllib.request.Request(
        base + "/mcp",
        data=b"x" * 1025,
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
    )
    try:
        urllib.request.urlopen(oversized, timeout=10)
        raise AssertionError("oversized MCP request unexpectedly accepted")
    except urllib.error.HTTPError as error:
        assert error.code == 413
    tools = rpc(2, "tools/list", {})["result"]["tools"]
    prompts = rpc(6, "prompts/list", {})["result"]["prompts"]
    assert "plomid.safe-query-workflow" in {prompt["name"] for prompt in prompts}
    prompt = rpc(7, "prompts/get", {"name": "plomid.safe-query-workflow"})
    assert "read-only" in prompt["result"]["messages"][0]["content"]["text"]
    assert tools and "plomid.sql.execute" not in {tool["name"] for tool in tools}
    safe = rpc(3, "tools/call", {"name": "plomid.capabilities", "arguments": {}})
    assert any(item["type"] == "text" for item in safe["result"]["content"])
    assert safe["result"]["structuredContent"]["prompts"] == ["plomid.safe-query-workflow"]
    if os.environ.get("PLOMID_MCP_SMOKE_DATABASE_URL"):
        live = rpc(8, "tools/call", {"name": "plomid.sql.query", "arguments": {"sql": "SELECT $1::integer AS answer", "parameters": ["42"]}})
        assert live["result"]["structuredContent"]["data"]["rows"] == [[42]]
    invalid = rpc(4, "tools/call", {"name": "plomid.sql.query", "arguments": {"sql": "SELECT 1; DROP TABLE synthetic"}})
    assert invalid.get("error") or invalid["result"].get("isError")
    unknown_tool = rpc(5, "tools/call", {"name": "conformance-fixture-tool-not-advertised", "arguments": {}})
    assert unknown_tool["error"]["code"] == -32602
    status, metrics = get("/metrics")
    assert status == 200 and "plomid_http_requests_total" in metrics
finally:
    process.send_signal(signal.SIGINT)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
    stderr = process.stderr.read()
    assert process.returncode == 0, f"server did not shut down cleanly ({process.returncode}): {stderr[-2000:]}"
    for secret in ("postgresql://", "synthetic-secret", "mcp-client-secret-canary", "Bearer", "DROP TABLE synthetic"):
        assert secret.lower() not in stderr.lower()

print("HTTP MCP smoke passed: live process, liveness/readiness, initialize, tool/prompt discovery, prompt retrieval, safe/invalid/unknown-tool calls, body limit, metrics, clean shutdown")
