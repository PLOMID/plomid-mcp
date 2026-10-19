#!/usr/bin/env python3
"""Run the capability-scoped MCP contract against a real release binary.

This is intentionally separate from the official MCP everything-server suite.
It exercises only PLOMID-MCP's advertised tools, prompt, and transports.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


class ContractFailure(AssertionError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractFailure(message)


def rpc_payload(request_id: int | None, method: str, params: dict[str, Any] | None = None) -> bytes:
    payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
    if request_id is not None:
        payload["id"] = request_id
    if params is not None:
        payload["params"] = params
    return json.dumps(payload).encode()


def response_json(response: urllib.response.addinfourl) -> dict[str, Any]:
    body = response.read().decode()
    if response.headers.get_content_type() == "text/event-stream":
        body = next(line[6:] for line in body.splitlines() if line.startswith("data: "))
    return json.loads(body)


class StdioClient:
    def __init__(self, binary: Path) -> None:
        self.process = subprocess.Popen(
            [str(binary), "--database-url", "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid", "stdio"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        require(self.process.stdin is not None and self.process.stdout is not None, "stdio pipes unavailable")

    def call(self, request_id: int, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        require(self.process.stdin is not None and self.process.stdout is not None, "stdio pipes unavailable")
        self.process.stdin.write(json.dumps(json.loads(rpc_payload(request_id, method, params))) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        require(line != "", f"stdio closed before response to {method}")
        return json.loads(line)

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        require(self.process.stdin is not None, "stdio input unavailable")
        self.process.stdin.write(json.dumps(json.loads(rpc_payload(None, method, params))) + "\n")
        self.process.stdin.flush()

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        self.process.wait(timeout=5)
        stderr = self.process.stderr.read() if self.process.stderr else ""
        require(self.process.returncode == 0, f"stdio shutdown failed: {stderr[-1000:]}")


def assert_initialize(result: dict[str, Any], expected_version: str) -> None:
    require(result.get("jsonrpc") == "2.0", "initialize response is not JSON-RPC 2.0")
    body = result.get("result")
    require(isinstance(body, dict), f"initialize failed: {result}")
    require(body.get("protocolVersion") == expected_version, f"unexpected negotiated version: {body}")
    require(body.get("serverInfo", {}).get("name") == "plomid-mcp", f"missing serverInfo: {body}")
    require(set(body.get("capabilities", {})) == {"tools", "prompts"}, f"capability drift: {body}")


def assert_tools_and_calls(call: Any) -> None:
    listed = call(2, "tools/list", {})
    tools = listed["result"]["tools"]
    names = {tool["name"] for tool in tools}
    expected = {
        "plomid.capabilities",
        "plomid.status",
        "plomid.sql.query",
        "plomid.sql.explain",
        "plomid.schema.list_databases",
        "plomid.schema.list_schemas",
        "plomid.schema.list_tables",
        "plomid.schema.describe_table",
    }
    require(names == expected, f"advertised tool set changed: {sorted(names)}")
    for tool in tools:
        schema = tool.get("inputSchema")
        require(isinstance(schema, dict) and schema.get("type") == "object", f"invalid schema: {tool}")
        require(1 <= len(tool["name"]) <= 128, f"invalid tool name: {tool['name']}")

    capabilities = call(3, "tools/call", {"name": "plomid.capabilities", "arguments": {}})
    structured = capabilities["result"].get("structuredContent")
    require(isinstance(structured, dict), f"capabilities lacks structured output: {capabilities}")
    require(set(structured["tools"]) == expected, "capabilities tool catalog disagrees with tools/list")
    require(structured["prompts"] == ["plomid.safe-query-workflow"], "capabilities prompt catalog disagrees")

    invalid_schema = call(4, "tools/call", {"name": "plomid.sql.query", "arguments": {"unexpected": True}})
    require(invalid_schema.get("error", {}).get("code") == -32602 or invalid_schema.get("result", {}).get("isError"), f"invalid arguments accepted: {invalid_schema}")
    unknown = call(5, "tools/call", {"name": "not-a-plomid-tool", "arguments": {}})
    require(unknown.get("error", {}).get("code") == -32602, f"unknown tool error changed: {unknown}")
    unsafe = call(6, "tools/call", {"name": "plomid.sql.query", "arguments": {"sql": "DROP TABLE users"}})
    require(unsafe.get("error", {}).get("code") == -32602 or unsafe.get("result", {}).get("isError"), f"unsafe SQL accepted: {unsafe}")


def assert_prompts(call: Any) -> None:
    listed = call(7, "prompts/list", {})
    prompts = listed["result"]["prompts"]
    require([prompt["name"] for prompt in prompts] == ["plomid.safe-query-workflow"], f"prompt catalog drift: {listed}")
    prompt = call(8, "prompts/get", {"name": "plomid.safe-query-workflow"})
    require("read-only" in prompt["result"]["messages"][0]["content"]["text"], f"prompt content invalid: {prompt}")
    missing = call(9, "prompts/get", {"name": "not-a-plomid-prompt"})
    require(missing.get("error", {}).get("code") == -32602, f"unknown prompt error changed: {missing}")


def run_stdio(binary: Path) -> None:
    client = StdioClient(binary)
    try:
        initialize = client.call(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "capability-suite", "version": "1"}})
        assert_initialize(initialize, "2025-11-25")
        client.notify("notifications/initialized")
        assert_tools_and_calls(client.call)
        assert_prompts(client.call)
        require(client.call(10, "ping", {}).get("result") is not None, "ping failed")
    finally:
        client.close()

    negotiated = subprocess.run(
        [str(binary), "--database-url", "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid", "stdio"],
        input=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "capability-suite", "version": "1"}}}) + "\n",
        text=True,
        capture_output=True,
        check=True,
    )
    assert_initialize(json.loads(negotiated.stdout), "2025-06-18")
    malformed = subprocess.run(
        [str(binary), "--database-url", "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid", "stdio"],
        input="not-json\n",
        text=True,
        capture_output=True,
    )
    require(malformed.returncode != 0 and not malformed.stdout, "malformed STDIO input was not rejected cleanly")
    print("capability suite: STDIO PASS")


class HttpClient:
    def __init__(self, base: str) -> None:
        self.base = base
        self.counter = 100

    def call(self, request_id: int, method: str, params: dict[str, Any] | None = None, *, headers: dict[str, str] | None = None) -> dict[str, Any]:
        request = urllib.request.Request(
            self.base + "/mcp",
            data=rpc_payload(request_id, method, params),
            headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream", **(headers or {})},
        )
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return response_json(response)
        except urllib.error.HTTPError as error:
            body = error.read().decode()
            return json.loads(body)

    def raw(self, path: str, payload: bytes, *, headers: dict[str, str] | None = None) -> tuple[int, bytes]:
        request = urllib.request.Request(self.base + path, data=payload, headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream", **(headers or {})})
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    def notify(self, method: str, params: dict[str, Any] | None = None) -> int:
        status, _ = self.raw("/mcp", rpc_payload(None, method, params))
        return status


def run_http(binary: Path) -> None:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    process = subprocess.Popen([str(binary), "--database-url", "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid", "--bind", f"127.0.0.1:{port}", "--max-request-body-bytes", "4096", "http"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    client = HttpClient(base)
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(base + "/healthz", timeout=1) as response:
                    require(response.status == 200, "health endpoint failed")
                    break
            except (OSError, urllib.error.URLError):
                time.sleep(0.1)
        else:
            raise ContractFailure("HTTP server did not become ready")

        initialize = client.call(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "capability-suite", "version": "1"}})
        assert_initialize(initialize, "2025-11-25")
        status, _ = client.raw("/mcp-invalid", rpc_payload(2, "ping", {}))
        require(status == 404, f"similar HTTP route was accepted: {status}")
        status, _ = client.raw("/mcp", rpc_payload(3, "ping", {}), headers={"Origin": "https://evil.example"})
        require(status == 403, f"invalid Origin was accepted: {status}")
        require(client.notify("notifications/initialized") in (200, 202, 204), "HTTP notification was not accepted")
        assert_tools_and_calls(client.call)
        assert_prompts(client.call)

        missing_meta = client.call(200, "server/discover", {})
        require(missing_meta.get("error", {}).get("code") == -32602, f"missing stateless metadata accepted: {missing_meta}")
        stateless = client.call(201, "server/discover", {"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}}, headers={"MCP-Protocol-Version": "2026-07-28", "Mcp-Method": "server/discover"})
        require(stateless.get("result", {}).get("capabilities", {}).get("tools") == {}, f"stateless discovery failed: {stateless}")

        malformed_status, malformed_body = client.raw("/mcp", b"not-json")
        require(malformed_status in (400, 415, 422), f"malformed HTTP JSON status unexpected: {malformed_status} {malformed_body!r}")
        oversized_status, _ = client.raw("/mcp", b"x" * 4097)
        require(oversized_status == 413, f"HTTP body limit failed: {oversized_status}")
        print("capability suite: HTTP PASS")
    finally:
        process.send_signal(signal.SIGINT)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        stderr = process.stderr.read() if process.stderr else ""
        require(process.returncode == 0, f"HTTP shutdown failed: {stderr[-1000:]}")
        for secret in ("synthetic-secret", "postgresql://", "capability-suite"):
            require(secret.lower() not in stderr.lower(), f"secret leaked to HTTP stderr: {secret}")


def run_http_auth(binary: Path) -> None:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    token = "capability-suite-token"
    digest = hashlib.sha256(token.encode()).hexdigest()
    config = f'''database_url = "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid"
bind = "127.0.0.1:{port}"

[auth]
required = true
tokens = [{{ token_sha256 = "{digest}", subject = "capability-suite", scopes = ["mcp:read", "sql:read", "schema:read"] }}]
'''
    with tempfile.NamedTemporaryFile("w", suffix=".toml") as settings:
        settings.write(config)
        settings.flush()
        process = subprocess.Popen([str(binary), "--config", settings.name, "http"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        base = f"http://127.0.0.1:{port}"
        client = HttpClient(base)
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                try:
                    with urllib.request.urlopen(base + "/healthz", timeout=1) as response:
                        require(response.status == 200, "authenticated server did not become ready")
                        break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.1)
            else:
                raise ContractFailure("authenticated server did not become ready")
            params = {"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}}
            headers = {"MCP-Protocol-Version": "2026-07-28", "Mcp-Method": "server/discover"}
            unauthenticated, _ = client.raw("/mcp", rpc_payload(1, "server/discover", params), headers=headers)
            require(unauthenticated == 401, f"missing bearer token was not rejected: {unauthenticated}")
            rejected, _ = client.raw("/mcp", rpc_payload(2, "server/discover", params), headers={**headers, "Authorization": "Bearer wrong-token"})
            require(rejected == 403, f"invalid bearer token was not rejected: {rejected}")
            authenticated = client.call(3, "server/discover", params, headers={**headers, "Authorization": f"Bearer {token}"})
            require(authenticated.get("result", {}).get("capabilities", {}).get("tools") == {}, f"authenticated discovery failed: {authenticated}")
            print("capability suite: HTTP authentication PASS")
        finally:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            stderr = process.stderr.read() if process.stderr else ""
            require(process.returncode == 0, f"authenticated HTTP shutdown failed: {stderr[-1000:]}")
            for secret in (token, digest, "synthetic-secret", "postgresql://"):
                require(secret.lower() not in stderr.lower(), f"authentication secret leaked to stderr: {secret}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path, nargs="?", default=Path("target/release/plomid-mcp"))
    args = parser.parse_args()
    require(args.binary.is_file(), f"release binary does not exist: {args.binary}")
    run_stdio(args.binary)
    run_http(args.binary)
    run_http_auth(args.binary)
    print("capability-scoped MCP conformance: PASS")


if __name__ == "__main__":
    main()
