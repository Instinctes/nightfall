#!/usr/bin/env python3
"""Fail-closed validation of Nightfall CI release bytes and signed provenance.

Scope: CI-built Linux, Windows and macOS release artifacts. Build provenance
only: no claim of audit, reproducibility, or Apple/Microsoft code signing.
"""
import argparse
import hashlib
from pathlib import Path
import re
import subprocess
import sys

REPO = "Instinctes/nightfall"
WORKFLOW = REPO + "/.github/workflows/release.yml"
VERSION = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?\Z")
SUM_LINE = re.compile(r"([0-9a-f]{64})  ([A-Za-z0-9._-]+)\Z")
PRODUCTS = ("nightfall-core", "nightfalld", "nightfall-wallet")
PLATFORMS = ("linux", "windows", "macos")


def expected_assets(tag, platform):
    if VERSION.fullmatch(tag) is None:
        raise ValueError("unsupported release tag")
    if platform not in PLATFORMS:
        raise ValueError("platform must be linux, windows or macos")
    version = tag[1:]
    if platform == "macos":
        # One DMG per architecture; the list keeps its historical name
        # SHA256SUMS-<version>.txt, without a platform suffix.
        binaries = tuple(f"NIGHTFALLCOIN-Core-{version}-macOS-{arch}.dmg"
                         for arch in ("arm64", "intel"))
        return binaries, f"SHA256SUMS-{version}.txt"
    ending = "linux-x64" if platform == "linux" else "windows-x64.exe"
    binaries = tuple(f"{name}-{version}-{ending}" for name in PRODUCTS)
    sums = f"SHA256SUMS-{version}-{platform}.txt"
    return binaries, sums


def safe_file(folder, name):
    path = folder / name
    if path.is_symlink() or not path.is_file() or path.name != name:
        raise ValueError(f"not a regular, non-symlink file: {name}")
    return path


def verify_checksums(folder, binaries, sums):
    manifest = safe_file(folder, sums)
    if manifest.stat().st_size > 4096:
        raise ValueError("checksum manifest too large")
    raw = manifest.read_bytes()
    if b"\r" in raw or b"\x00" in raw:
        raise ValueError("checksum manifest has invalid control bytes")
    try:
        lines = raw.decode("ascii").splitlines()
    except UnicodeDecodeError as exc:
        raise ValueError("checksum manifest not ASCII") from exc
    parsed = {}
    for line in lines:
        match = SUM_LINE.fullmatch(line)
        if match is None:
            raise ValueError("invalid checksum line")
        digest, name = match.groups()
        if name in parsed:
            raise ValueError("duplicate asset checksum")
        parsed[name] = digest
    if set(parsed) != set(binaries):
        raise ValueError("checksum manifest has missing or unexpected assets")
    for name in binaries:
        h = hashlib.sha256()
        with safe_file(folder, name).open("rb") as fp:
            for chunk in iter(lambda: fp.read(1024 * 1024), b""):
                h.update(chunk)
        if h.hexdigest() != parsed[name]:
            raise ValueError(f"checksum mismatch: {name}")


def verify_provenance(folder, files, tag, run=subprocess.run):
    for name in files:
        cmd = ["gh", "attestation", "verify", str(safe_file(folder, name)),
               "--repo", REPO, "--signer-workflow", WORKFLOW,
               "--source-ref", "refs/tags/" + tag]
        result = run(cmd, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise ValueError(f"missing or invalid signed provenance: {name}; "
                             f"{(result.stderr or '')[:400]}")


def verify(folder, tag, platform, run=subprocess.run):
    folder = Path(folder)
    binaries, sums = expected_assets(tag, platform)
    needed = set(binaries) | {sums}
    actual = {p.name for p in folder.iterdir()}
    if actual != needed:
        raise ValueError(f"unexpected/missing release assets: {sorted(actual ^ needed)}")
    verify_checksums(folder, binaries, sums)
    verify_provenance(folder, [*binaries, sums], tag, run=run)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--tag", required=True)
    p.add_argument("--platform", choices=PLATFORMS, required=True)
    p.add_argument("--directory", required=True)
    args = p.parse_args()
    try:
        verify(args.directory, args.tag, args.platform)
    except (ValueError, OSError) as exc:
        print("RELEASE REJECTED:", exc, file=sys.stderr)
        return 1
    print("PASS: checksums and GitHub Sigstore build provenance verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
