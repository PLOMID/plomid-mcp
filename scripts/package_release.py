#!/usr/bin/env python3
"""Build a platform-appropriate distribution with SBOM and SHA-256 checksum."""

import argparse
import hashlib
import subprocess
import tarfile
import zipfile
from pathlib import Path


root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--binary", type=Path, default=root / "target/release/plomid-mcp")
parser.add_argument("--sbom", type=Path, default=root / "plomid-mcp.cdx.json")
parser.add_argument("--target", help="Rust target triple; defaults to the local rustc host")
parser.add_argument("--output", type=Path, default=root / "dist")
args = parser.parse_args()
binary = args.binary.resolve()
sbom = args.sbom.resolve()
version = subprocess.check_output([str(binary), "--version"], text=True).strip().split()[-1]
target = args.target or subprocess.check_output(["rustc", "-vV"], text=True).split("host: ", 1)[1].splitlines()[0]
name = f"plomid-mcp-{version}-{target}"
is_windows = "windows" in target
archive = args.output.resolve() / (f"{name}.zip" if is_windows else f"{name}.tar.gz")
args.output.resolve().mkdir(parents=True, exist_ok=True)

binary_name = "plomid-mcp.exe" if is_windows else "plomid-mcp"
members = {
    binary_name: binary,
    "plomid-mcp.cdx.json": sbom,
    "README.md": root / "README.md",
    "LICENSE": root / "LICENSE",
    "NOTICE": root / "NOTICE",
    "plomid-mcp.example.toml": root / "plomid-mcp.example.toml",
}
for member, path in members.items():
    if not path.is_file():
        raise SystemExit(f"required package input is missing: {path}")

if is_windows:
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as package:
        for member, path in members.items():
            package.write(path, arcname=f"{name}/{member}")
else:
    with tarfile.open(archive, "w:gz", format=tarfile.PAX_FORMAT) as package:
        for member, path in members.items():
            package.add(path, arcname=f"{name}/{member}", recursive=False)

digest = hashlib.sha256(archive.read_bytes()).hexdigest()
checksum = archive.with_suffix(archive.suffix + ".sha256")
checksum.write_text(f"{digest}  {archive.name}\n", encoding="ascii")
print(f"package={archive}")
print(f"target={target}")
print(f"version={version}")
print(f"sha256={digest}")
