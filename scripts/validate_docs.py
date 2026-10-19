#!/usr/bin/env python3
"""Validate the checked-in Mintlify navigation against real MDX files."""

import json
from pathlib import Path


root = Path(__file__).resolve().parents[1]
site = root / "mcp-docs"

config = json.loads((site / "docs.json").read_text(encoding="utf-8"))


def collect_pages(node):
    pages = []
    if isinstance(node, list):
        for item in node:
            pages.extend(collect_pages(item))
    elif isinstance(node, dict):
        if isinstance(node.get("page"), str):
            pages.append(node["page"])
        for page in node.get("pages", []):
            if isinstance(page, str):
                pages.append(page)
            else:
                pages.extend(collect_pages(page))
        for key in ("groups", "tabs", "anchors"):
            pages.extend(collect_pages(node.get(key, [])))
    return pages


navigation = config["navigation"]
pages = collect_pages(navigation)
missing = [page for page in pages if not (site / f"{page}.mdx").is_file()]
if missing:
    raise SystemExit("missing Mintlify pages: " + ", ".join(missing))

required = {
    "index", "getting-started/requirements", "getting-started/installation",
    "getting-started/quickstart", "getting-started/first-connection",
    "getting-started/troubleshooting", "architecture/overview",
    "architecture/trust-boundaries", "configuration/reference",
    "integration/overview", "integration/stdio", "integration/http",
    "integration/client-compatibility", "tools/catalog",
    "tools/query-contract", "tools/write-contract", "operations/overview",
    "operations/runbooks", "security/model", "security/limitations",
    "release/platforms", "release/verification",
}
missing_required = sorted(required - set(pages))
if missing_required:
    raise SystemExit("required MCP pages are missing from navigation: " + ", ".join(missing_required))

print(f"validated {len(pages)} Mintlify navigation pages in mcp-docs")
