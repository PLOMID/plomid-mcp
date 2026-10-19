#!/usr/bin/env python3
"""Reproducible non-database STDIO startup and MCP call baseline."""

import json
import platform
import statistics
import subprocess
import sys
import time
from pathlib import Path

binary = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("target/release/plomid-mcp")
samples = int(sys.argv[2]) if len(sys.argv) > 2 else 30
initialize_ms = []
safe_call_ms = []
startup_ms = []

for sample in range(samples):
    started = time.perf_counter()
    process = subprocess.Popen(
        [str(binary), "--database-url", "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid", "stdio"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "local-benchmark", "version": "1"}}}) + "\n")
    process.stdin.flush()
    initialize = json.loads(process.stdout.readline())
    init_done = time.perf_counter()
    assert initialize["result"]["protocolVersion"] == "2025-11-25"
    process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "plomid.capabilities", "arguments": {}}}) + "\n")
    process.stdin.flush()
    safe_result = json.loads(process.stdout.readline())
    call_done = time.perf_counter()
    assert not safe_result.get("error")
    startup_ms.append((init_done - started) * 1000)
    initialize_ms.append((init_done - started) * 1000)
    safe_call_ms.append((call_done - init_done) * 1000)
    process.stdin.close()
    process.wait(timeout=5)
    assert process.returncode == 0, process.stderr.read()


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int((len(ordered) - 1) * fraction))]


print(json.dumps({
    "scope": "local synthetic backend-unavailable configuration; STDIO only; not a database benchmark",
    "host": platform.platform(),
    "python": platform.python_version(),
    "binary": str(binary),
    "sample_count": samples,
    "metrics_ms": {
        "startup_to_initialize_response": {"median": statistics.median(startup_ms), "p95": percentile(startup_ms, 0.95)},
        "initialize_round_trip_including_process_start": {"median": statistics.median(initialize_ms), "p95": percentile(initialize_ms, 0.95)},
        "capabilities_tool_call": {"median": statistics.median(safe_call_ms), "p95": percentile(safe_call_ms, 0.95)},
    },
}, indent=2))
