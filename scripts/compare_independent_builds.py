#!/usr/bin/env python3
"""Offline, untrusted build-receipt comparison. Never invokes a build or network."""
import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

SCHEMA = "nightfall-build-comparison-v1"
BINARIES = frozenset(("nightfalld", "nightfall-wallet", "nightfall-core"))
KEYS = frozenset(("schema", "source_commit", "cargo_lock_sha256", "rustc", "cargo", "host", "profile", "artifacts"))
INPUTS = ("source_commit", "cargo_lock_sha256", "rustc", "cargo", "host", "profile")
HEX64 = re.compile(r"[0-9a-f]{64}\Z")
SAFE_TEXT = re.compile(r"[A-Za-z0-9 ._()+-]{1,120}\Z")
HOST = re.compile(r"[a-zA-Z0-9_.-]{1,100}\Z")
MAX_RECEIPT = 8192


class BuildEvidenceError(ValueError):
    pass


def regular_file(path):
    p = Path(path)
    if p.is_symlink() or not p.is_file():
        raise BuildEvidenceError("regular non-symlink file required")
    return p


def digest_file(path):
    p = regular_file(path)
    before = p.stat()
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    after = p.stat()
    if any(getattr(before, k) != getattr(after, k) for k in
           ("st_dev", "st_ino", "st_size", "st_mtime_ns")):
        raise BuildEvidenceError("file changed during hashing")
    return {"sha256": h.hexdigest(), "bytes": after.st_size}


def command(args, cwd=None):
    try:
        p = subprocess.run(args, cwd=cwd, capture_output=True, text=True,
                           check=False, timeout=10)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise BuildEvidenceError("required local tool unavailable") from exc
    if p.returncode != 0 or len(p.stdout) > 2048:
        raise BuildEvidenceError("local identity command failed")
    return p.stdout.strip()


def record(repo, binaries):
    repo = Path(repo)
    regular_file(repo / "Cargo.lock")
    sha = command(["git", "rev-parse", "HEAD"], cwd=repo)
    if not HEX64.fullmatch(sha):
        raise BuildEvidenceError("invalid checked-out Git commit")
    if command(["git", "status", "--porcelain", "--untracked-files=normal"], cwd=repo):
        raise BuildEvidenceError("source checkout is not clean")
    rv = command(["rustc", "-vV"])
    lines = rv.splitlines()
    rustc = next((x for x in lines if x.startswith("rustc ")), "")
    host = next((x[6:].strip() for x in lines if x.startswith("host: ")), "")
    cargo = command(["cargo", "--version"])
    if not SAFE_TEXT.fullmatch(rustc) or not SAFE_TEXT.fullmatch(cargo) or not HOST.fullmatch(host):
        raise BuildEvidenceError("invalid compiler identity")
    if not binaries or len(binaries) > len(BINARIES):
        raise BuildEvidenceError("provide one to three Nightfall binaries")
    artifacts = {}
    for item in binaries:
        if "=" not in item:
            raise BuildEvidenceError("binary syntax must be NAME=PATH")
        name, file_path = item.split("=", 1)
        if name not in BINARIES or name in artifacts or not file_path:
            raise BuildEvidenceError("invalid or duplicate binary name")
        artifacts[name] = digest_file(file_path)
    obj = {"schema": SCHEMA, "source_commit": sha,
           "cargo_lock_sha256": digest_file(repo / "Cargo.lock")["sha256"],
           "rustc": rustc, "cargo": cargo, "host": host, "profile": "release",
           "artifacts": artifacts}
    validate(obj)
    return obj


def unique_pairs(pairs):
    out = {}
    for name, value in pairs:
        if name in out:
            raise BuildEvidenceError("duplicate JSON field")
        out[name] = value
    return out


def validate(r):
    if type(r) is not dict or set(r) != KEYS or r["schema"] != SCHEMA:
        raise BuildEvidenceError("unsupported or non-canonical receipt schema")
    if not all(type(r[k]) is str for k in INPUTS):
        raise BuildEvidenceError("invalid build identity types")
    if not HEX64.fullmatch(r["source_commit"]) or not HEX64.fullmatch(r["cargo_lock_sha256"]):
        raise BuildEvidenceError("invalid source or dependency hash")
    if not SAFE_TEXT.fullmatch(r["rustc"]) or not SAFE_TEXT.fullmatch(r["cargo"]):
        raise BuildEvidenceError("invalid toolchain identity")
    if not HOST.fullmatch(r["host"]) or r["profile"] != "release":
        raise BuildEvidenceError("invalid target or build profile")
    a = r["artifacts"]
    if type(a) is not dict or not a or not set(a).issubset(BINARIES):
        raise BuildEvidenceError("invalid artifact map")
    for item in a.values():
        if type(item) is not dict or set(item) != {"sha256", "bytes"}:
            raise BuildEvidenceError("invalid artifact schema")
        if type(item["sha256"]) is not str or not HEX64.fullmatch(item["sha256"]):
            raise BuildEvidenceError("invalid binary digest")
        if type(item["bytes"]) is not int or not 0 < item["bytes"] < (1 << 50):
            raise BuildEvidenceError("invalid binary size")
    return r


def read_receipt(path):
    f = regular_file(path)
    if f.stat().st_size > MAX_RECEIPT:
        raise BuildEvidenceError("oversized receipt")
    try:
        raw = f.read_bytes()
        return validate(json.loads(raw.decode("utf-8"), object_pairs_hook=unique_pairs,
                                   parse_constant=lambda v: (_ for _ in ()).throw(
                                       BuildEvidenceError("invalid numeric constant"))))
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise BuildEvidenceError("malformed receipt JSON") from exc


def compare(a, b):
    validate(a)
    validate(b)
    different_inputs = [key for key in INPUTS if a[key] != b[key]]
    if different_inputs or set(a["artifacts"]) != set(b["artifacts"]):
        return "INCOMPARABLE", 2, different_inputs + (["artifact_names"] if set(a["artifacts"]) != set(b["artifacts"]) else [])
    diffs = [name for name in sorted(a["artifacts"]) if a["artifacts"][name] != b["artifacts"][name]]
    return ("DIFFERENT_BYTES", 1, diffs) if diffs else ("MATCHING_BYTES", 0, [])


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="action", required=True)
    rec = sub.add_parser("record", help="read-only local native release artifact receipt")
    rec.add_argument("--repo", required=True)
    rec.add_argument("--binary", action="append", required=True, metavar="NAME=PATH")
    cmp = sub.add_parser("compare", help="offline compare two untrusted receipts")
    cmp.add_argument("first")
    cmp.add_argument("second")
    args = p.parse_args(argv)
    try:
        if args.action == "record":
            print(json.dumps(record(args.repo, args.binary), sort_keys=True, separators=(",", ":")))
            return 0
        result, status, different = compare(read_receipt(args.first), read_receipt(args.second))
        print(json.dumps({"assessment": result, "different_fields": different}, sort_keys=True))
        return status
    except (BuildEvidenceError, OSError) as exc:
        print("BUILD EVIDENCE REJECTED: " + str(exc), file=sys.stderr)
        return 3


if __name__ == "__main__":
    sys.exit(main())
