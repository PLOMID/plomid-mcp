#!/usr/bin/env python3
"""Verify package checksum, safe archive members, executable version, and MCP smoke."""

import hashlib
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path, PurePosixPath, PureWindowsPath


def assert_safe_member(name: str) -> None:
    normalized = name.replace("\\", "/")
    posix_path = PurePosixPath(normalized)
    windows_path = PureWindowsPath(name)
    assert not posix_path.is_absolute() and not windows_path.is_absolute(), f"unsafe archive path: {name}"
    assert not windows_path.drive and ".." not in posix_path.parts, f"unsafe archive path: {name}"


if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "--stdio-only"):
    raise SystemExit("usage: validate_package.py PACKAGE [--stdio-only]")
archive = Path(sys.argv[1]).resolve()
stdio_only = len(sys.argv) == 3
checksum_file = archive.with_suffix(archive.suffix + ".sha256")
expected = checksum_file.read_text(encoding="ascii").split()[0]
actual = hashlib.sha256(archive.read_bytes()).hexdigest()
assert actual == expected, "archive checksum mismatch"

with tempfile.TemporaryDirectory(prefix="plomid-mcp-package-") as destination:
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as package:
            names = package.namelist()
            for info in package.infolist():
                assert_safe_member(info.filename)
                unix_mode = info.external_attr >> 16
                assert stat.S_IFMT(unix_mode) != stat.S_IFLNK, f"unsupported archive link: {info.filename}"
            binary_name = "plomid-mcp.exe"
            binaries = [name for name in names if Path(name).name == binary_name]
            assert len(binaries) == 1
            package.extractall(destination)
    else:
        with tarfile.open(archive, "r:gz") as package:
            members = package.getmembers()
            for member in members:
                assert_safe_member(member.name)
                assert not member.issym() and not member.islnk(), f"unsupported archive link: {member.name}"
            binaries = [member.name for member in members if Path(member.name).name == "plomid-mcp"]
            assert len(binaries) == 1
            package.extractall(destination, filter="data")

    binary = Path(destination) / binaries[0]
    package_root = Path(binaries[0]).parent
    expected_files = {"plomid-mcp.cdx.json", "README.md", "LICENSE", "NOTICE", "plomid-mcp.example.toml"}
    archived_files = {Path(name).name for name in names} if zipfile.is_zipfile(archive) else {Path(member.name).name for member in members}
    assert expected_files.issubset(archived_files)
    assert binary.is_file()
    if os.name != "nt" and binary.suffix != ".exe":
        assert binary.stat().st_mode & 0o111, "packaged binary is not executable"
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    config = Path(destination) / package_root / "plomid-mcp.example.toml"
    sbom = json.loads((Path(destination) / package_root / "plomid-mcp.cdx.json").read_text(encoding="utf-8"))
    assert sbom["bomFormat"] == "CycloneDX" and sbom["components"]
    checked_config = subprocess.run([str(binary), "--config", str(config), "validate"], capture_output=True, text=True)
    assert checked_config.returncode == 0, checked_config.stderr
    subprocess.run([sys.executable, "scripts/mcp_stdio_smoke.py", str(binary)], check=True)
    if not stdio_only:
        subprocess.run([sys.executable, "scripts/mcp_capability_conformance.py", str(binary)], check=True)

print(f"package validated: {archive.name}; sha256={actual}; {version}")
