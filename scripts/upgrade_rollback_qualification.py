#!/usr/bin/env python3
"""Qualify an isolated package upgrade, interrupted upgrade, and rollback.

PLOMID stores configuration outside the executable and does not maintain a
migration-managed application data directory.  The test therefore verifies
both the packaged executable and preservation of an operator-owned config
and state file.  A real previous package is mandatory; this command refuses
to treat a copy of the current package as an upgrade fixture.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path) -> str:
    checksum = path.with_suffix(path.suffix + ".sha256")
    if not checksum.is_file():
        raise SystemExit(f"missing checksum for {path}; produce it with scripts/package_release.py")
    expected = checksum.read_text(encoding="ascii").split()[0]
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if expected != actual:
        raise SystemExit(f"checksum mismatch for {path}")
    return actual


def extract(archive: Path, destination: Path) -> tuple[Path, str]:
    digest(archive)
    with tarfile.open(archive, "r:gz") as package:
        members = package.getmembers()
        for member in members:
            member_path = Path(member.name)
            if member_path.is_absolute() or ".." in member_path.parts or member.issym() or member.islnk():
                raise SystemExit(f"unsafe or unsupported package member: {member.name}")
        package.extractall(destination, filter="data")
    binary_members = [member for member in members if Path(member.name).name == "plomid-mcp"]
    if len(binary_members) != 1:
        raise SystemExit(f"package must contain one plomid-mcp executable: {archive}")
    binary = destination / binary_members[0].name
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    return binary, version


def run(binary: Path, *args: str) -> None:
    subprocess.run([str(binary), *args], cwd=ROOT, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def smoke(binary: Path) -> None:
    run(binary, "--config", str(binary.parent / "plomid-mcp.example.toml"), "validate")
    protocol_smoke(binary)


def protocol_smoke(binary: Path) -> None:
    subprocess.run(["python3", str(ROOT / "scripts/mcp_stdio_smoke.py"), str(binary)], check=True)
    subprocess.run(["python3", str(ROOT / "scripts/mcp_capability_conformance.py"), str(binary)], check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--previous", type=Path, required=True, help="previous supported package and adjacent .sha256")
    parser.add_argument("--current", type=Path, required=True, help="current package and adjacent .sha256")
    args = parser.parse_args()
    previous = args.previous.resolve()
    current = args.current.resolve()
    if previous == current or previous.read_bytes() == current.read_bytes():
        raise SystemExit("previous package must be a distinct released package, not a copy of current")

    with tempfile.TemporaryDirectory(prefix="plomid-mcp-upgrade-") as temp:
        workspace = Path(temp)
        old_root = workspace / "old"
        new_root = workspace / "new"
        old_root.mkdir()
        new_root.mkdir()
        old_binary, old_version = extract(previous, old_root)
        new_binary, new_version = extract(current, new_root)
        if old_version == new_version:
            raise SystemExit(f"previous and current versions are identical ({old_version}); a real prior package is required")

        # Deployment-owned files live outside the package and must survive a replacement.
        deploy = workspace / "deployment"
        deploy.mkdir()
        config = deploy / "plomid-mcp.toml"
        state = deploy / "operator-state.json"
        config.write_text('database_url = "postgresql://reader:synthetic-secret@127.0.0.1:1/plomid"\n', encoding="utf-8")
        state.write_text(json.dumps({"schema_version": 1, "sentinel": "upgrade-rollback"}) + "\n", encoding="utf-8")
        config_before = config.read_bytes()
        state_before = state.read_bytes()

        # Initial installation and the previous release must both be executable.
        run(old_binary, "--config", str(config), "validate")
        smoke(old_binary)

        active = deploy / "plomid-mcp"
        previous_active = deploy / "plomid-mcp.previous"
        shutil.copy2(old_binary, active)
        shutil.copy2(old_binary, previous_active)
        run(active, "--version")

        # Normal upgrade: replace only the executable, retaining operator files.
        shutil.copy2(new_binary, active)
        if active.read_bytes() != new_binary.read_bytes():
            raise SystemExit("upgrade did not install the current executable")
        run(active, "--version")
        run(active, "--config", str(config), "validate")
        if config.read_bytes() != config_before or state.read_bytes() != state_before:
            raise SystemExit("upgrade changed deployment configuration or persistent state")
        smoke(new_binary)

        # Interrupted upgrade simulation: place a bad executable, then restore the backup.
        interrupted = deploy / "plomid-mcp.incoming"
        interrupted.write_bytes(b"incomplete package")
        os.replace(interrupted, active)
        shutil.copy2(previous_active, active)
        if active.read_bytes() != old_binary.read_bytes():
            raise SystemExit("rollback did not restore the previous executable")
        run(active, "--version")
        run(active, "--config", str(config), "validate")
        if config.read_bytes() != config_before or state.read_bytes() != state_before:
            raise SystemExit("rollback changed deployment configuration or persistent state")

        # A rollback is complete only when the restored package still passes its protocol smoke.
        protocol_smoke(active)
        print(f"upgrade/rollback qualification passed: {old_version} -> {new_version} -> {old_version}")


if __name__ == "__main__":
    main()
