#!/usr/bin/env python3
"""Run authorized, read-only qualification against a real PLOMID MCP endpoint.

The endpoint and optional bearer token must be supplied explicitly. This script
never mutates data and does not claim results unless it actually connects.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import socket
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


def request(url: str, request_id: int, method: str, params: dict[str, Any], token: str | None, timeout: float) -> dict[str, Any]:
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    body = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}).encode()
    http_request = urllib.request.Request(url, data=body, headers=headers)
    with urllib.request.urlopen(http_request, timeout=timeout) as response:
        payload = response.read().decode()
        if response.headers.get_content_type() == "text/event-stream":
            payload = next(line[6:] for line in payload.splitlines() if line.startswith("data: "))
        return json.loads(payload)


def result(response: dict[str, Any], operation: str) -> dict[str, Any]:
    if "error" in response:
        raise RuntimeError(f"{operation} returned JSON-RPC error: {response['error']}")
    return response["result"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=os.environ.get("PLOMID_MCP_QUALIFICATION_URL"), required=not os.environ.get("PLOMID_MCP_QUALIFICATION_URL"))
    parser.add_argument("--token", default=os.environ.get("PLOMID_MCP_QUALIFICATION_TOKEN"))
    parser.add_argument("--limited-token", default=os.environ.get("PLOMID_MCP_QUALIFICATION_LIMITED_TOKEN"), help="optional token that must not have sql:read")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--timeout-query-ms", type=int, default=50)
    parser.add_argument("--cancellation-timeout-ms", type=int, default=100)
    args = parser.parse_args()

    base = args.url.rstrip("/")
    if not base.endswith("/mcp"):
        base += "/mcp"
    request(base, 1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "plomid-live-qualification", "version": "1"}}, args.token, 10)
    status = result(request(base, 2, "tools/call", {"name": "plomid.status", "arguments": {}}, args.token, 10), "plomid.status")
    if not status.get("database_configured"):
        raise RuntimeError("live endpoint reports an unconfigured database")

    query_args = {"name": "plomid.sql.query", "arguments": {"sql": "SELECT $1::integer AS answer", "parameters": ["42"], "row_limit": 1, "timeout_ms": 5000}}
    query = result(request(base, 3, "tools/call", query_args, args.token, 10), "parameterized query")
    if query.get("isError"):
        raise RuntimeError(f"parameterized query failed: {query}")

    invalid = request(base, 4, "tools/call", {"name": "plomid.sql.query", "arguments": {"sql": "SELECT FROM", "timeout_ms": 5000}}, args.token, 10)
    if not invalid.get("result", {}).get("isError") and "error" not in invalid:
        raise RuntimeError(f"invalid SQL unexpectedly succeeded: {invalid}")

    if args.limited_token:
        denied = request(base, 5, "tools/call", query_args, args.limited_token, 10)
        if not denied.get("result", {}).get("isError") and "error" not in denied:
            raise RuntimeError(f"limited token unexpectedly executed SQL: {denied}")

    def workload(index: int) -> dict[str, Any]:
        return request(base, 100 + index, "tools/call", {"name": "plomid.sql.query", "arguments": {"sql": "SELECT $1::integer AS worker", "parameters": [str(index)], "row_limit": 1, "timeout_ms": 5000}}, args.token, 10)

    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        responses = list(executor.map(workload, range(args.workers)))
    if any("error" in response or response.get("result", {}).get("isError") for response in responses):
        raise RuntimeError("one or more bounded workload requests failed")

    timeout_args = {"name": "plomid.sql.query", "arguments": {"sql": "SELECT pg_sleep(2)", "row_limit": 1, "timeout_ms": args.timeout_query_ms}}
    timeout_result = request(base, 300, "tools/call", timeout_args, args.token, 10)
    if not timeout_result.get("result", {}).get("isError") and "error" not in timeout_result:
        raise RuntimeError(f"timeout query unexpectedly succeeded: {timeout_result}")
    recovery = result(request(base, 301, "tools/call", {"name": "plomid.status", "arguments": {}}, args.token, 10), "post-timeout recovery")
    if not recovery.get("database_configured"):
        raise RuntimeError("database configuration disappeared after timeout")

    # Disconnect a bounded read while it is in flight.  This exercises the
    # transport cancellation path without issuing a mutation.
    cancellation_args = {"name": "plomid.sql.query", "arguments": {"sql": "SELECT pg_sleep(2)", "row_limit": 1, "timeout_ms": 5000}}
    try:
        request(base, 302, "tools/call", cancellation_args, args.token, args.cancellation_timeout_ms / 1000)
    except (TimeoutError, socket.timeout, urllib.error.URLError):
        pass
    else:
        raise RuntimeError("cancellation probe completed instead of disconnecting within its bound")
    post_cancel = result(request(base, 303, "tools/call", {"name": "plomid.status", "arguments": {}}, args.token, 10), "post-cancellation recovery")
    if not post_cancel.get("database_configured"):
        raise RuntimeError("database configuration disappeared after cancellation")

    unavailable_port = 1
    parsed = urllib.parse.urlparse(base)
    if parsed.hostname in {"127.0.0.1", "localhost", "::1"}:
        unavailable = urllib.parse.urlunparse(parsed._replace(port=unavailable_port))
        try:
            request(unavailable, 999, "tools/call", {"name": "plomid.status", "arguments": {}}, args.token, 1)
        except (OSError, urllib.error.URLError, socket.timeout):
            pass
        else:
            raise RuntimeError("connectivity-failure probe unexpectedly reached a service on port 1")

    limited = "PASS" if args.limited_token else "NOT_CONFIGURED"
    print(f"authorized live PLOMID qualification passed: workers={args.workers}, workload_seconds={time.monotonic() - started:.3f}, timeout_recovery=PASS, cancellation_recovery=PASS, limited_token={limited}")


if __name__ == "__main__":
    main()
