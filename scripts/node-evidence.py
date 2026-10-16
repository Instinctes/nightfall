#!/usr/bin/env python3
"""NIGHTFALL Node Evidence: local, redacted, falsifiable operator evidence.

Read-only RPC status and verify_supply; intentionally not a consensus verifier.
"""
import argparse
import hashlib
import json
import os
import socket
import sys
import time
from pathlib import Path

GENESIS = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de"
MAX_DARKS = 90_000_000 * 100_000_000
MAX_BYTES = 65536
ALLOWED = ("network", "genesis", "protocol_version", "wire_version", "tip", "tip_height",
           "blocks", "total_work", "minted", "burned_fees", "circulating", "loading",
           "supply_invariant_ok", "live_peers", "best_peer_height", "stalled_on_fork",
           "reorg_in_flight", "mining", "mining_idle")


class EvidenceError(Exception):
    pass


def unique_pairs(pairs):
    output = {}
    for key, value in pairs:
        if key in output:
            raise EvidenceError("duplicate JSON key")
        output[key] = value
    return output


def parse_json(data):
    try:
        return json.loads(data, object_pairs_hook=unique_pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(EvidenceError("nonfinite JSON")))
    except (ValueError, UnicodeError, TypeError) as ex:
        raise EvidenceError("invalid RPC JSON") from ex


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=True,
                      separators=(",", ":"), allow_nan=False).encode("ascii")


def rpc(port, method, req_id):
    request = canonical({"method": method, "params": {}, "id": req_id}) + b"\n"
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=4) as conn:
            conn.settimeout(6)
            conn.sendall(request)
            chunks = bytearray()
            while len(chunks) <= MAX_BYTES:
                b = conn.recv(min(4096, MAX_BYTES + 1 - len(chunks)))
                if not b:
                    raise EvidenceError("RPC ended without newline")
                chunks.extend(b)
                if b"\n" in b:
                    break
            if len(chunks) > MAX_BYTES or b"\n" not in chunks:
                raise EvidenceError("RPC response is oversized")
        envelope = parse_json(bytes(chunks).split(b"\n", 1)[0].decode("utf-8", "strict"))
    except (OSError, UnicodeError) as ex:
        raise EvidenceError("local RPC unavailable or invalid") from ex
    if (not isinstance(envelope, dict) or envelope.get("id") != req_id or
            envelope.get("error") is not None or not isinstance(envelope.get("result"), dict)):
        raise EvidenceError("RPC ID, error or response shape invalid")
    return envelope["result"]


def uint(value, label):
    if type(value) is not int or value < 0 or value > (1 << 128) - 1:
        raise EvidenceError("invalid unsigned integer: " + label)
    return value


def hash64(value, label):
    if not isinstance(value, str) or len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
        raise EvidenceError("invalid hash: " + label)
    return value


def check_status(source):
    if not isinstance(source, dict):
        raise EvidenceError("status not an object")
    s = {k: source[k] for k in ALLOWED if k in source}
    for k in ALLOWED:
        if k not in s:
            raise EvidenceError("missing status field: " + k)
    if s["loading"] is True:
        raise EvidenceError("node still loading: retry after sync")
    if s["network"] != "mainnet" or hash64(s["genesis"], "genesis") != GENESIS:
        raise EvidenceError("wrong network or genesis")
    if uint(s["protocol_version"], "protocol_version") != 8 or uint(s["wire_version"], "wire_version") != 6:
        raise EvidenceError("incompatible protocol/wire version")
    hash64(s["tip"], "tip")
    for k in ("tip_height", "blocks", "minted", "burned_fees", "circulating", "live_peers", "best_peer_height"):
        uint(s[k], k)
    if (not isinstance(s["total_work"], str) or not s["total_work"].isdigit() or
            len(s["total_work"]) > 39 or int(s["total_work"]) <= 0):
        raise EvidenceError("invalid cumulative work")
    for k in ("loading", "supply_invariant_ok", "stalled_on_fork", "reorg_in_flight", "mining"):
        if type(s[k]) is not bool:
            raise EvidenceError("invalid boolean: " + k)
    if s["mining_idle"] not in ("", "behind a peer", "no template"):
        raise EvidenceError("invalid mining_idle")
    if s["blocks"] != s["tip_height"] + 1:
        raise EvidenceError("blocks/height mismatch")
    if s["minted"] > MAX_DARKS or s["burned_fees"] > s["minted"]:
        raise EvidenceError("reported monetary bounds violated")
    if s["circulating"] != s["minted"] - s["burned_fees"]:
        raise EvidenceError("reported circulating amount inconsistent")
    return s


def check_supply(result, s):
    if type(result.get("ok")) is not bool or result["ok"] is not True:
        raise EvidenceError("node's supply verification failed")
    if uint(result.get("circulating_darks"), "circulating_darks") != s["circulating"]:
        raise EvidenceError("supply RPC disagrees with status")


def digest(body):
    return hashlib.sha256(canonical(body)).hexdigest()


def seal(body):
    return {"payload": body, "sha256": digest(body)}


def verify_receipt(obj):
    if not isinstance(obj, dict) or set(obj) != {"payload", "sha256"} or not isinstance(obj["payload"], dict):
        raise EvidenceError("invalid receipt container")
    if obj["sha256"] != digest(obj["payload"]):
        raise EvidenceError("receipt hash mismatch")
    body = obj["payload"]
    if (body.get("schema") != "nightfall-node-evidence-v1" or
            body.get("scope") != "untrusted-local-rpc-claims-not-consensus-proof" or
            not isinstance(body.get("samples"), list) or len(body["samples"]) != 2):
        raise EvidenceError("invalid receipt schema")
    for sample in body["samples"]:
        s = check_status(sample)
        if sample != s:  # disallow arbitrary strings/IPs/keys in receipts
            raise EvidenceError("receipt contains unexpected data")
    if body.get("checks") != ["status", "verify_supply"]:
        raise EvidenceError("invalid receipt checks")
    if body.get("assessment") not in ("OBSERVED", "CAUTION"):
        raise EvidenceError("invalid receipt assessment")
    if set(body) != {"schema", "scope", "samples", "checks", "assessment", "warnings"}:
        raise EvidenceError("unexpected receipt fields")
    if not isinstance(body["warnings"], list) or any(x not in WARNINGS for x in body["warnings"]):
        raise EvidenceError("invalid receipt warnings")
    return body


WARNINGS = {
    "node still loading", "no live peers", "peer ahead of local tip", "fork recovery active",
    "node supply check not ready", "mining enabled while behind with no explicit idle hold",
    "tip changed at same height", "cumulative work decreased", "tip height decreased",
}


def assess(first, second):
    warnings = set()
    if first["genesis"] != second["genesis"] or first["network"] != second["network"]:
        raise EvidenceError("network identity changed between samples")
    for s in (first, second):
        if s["loading"]:
            warnings.add("node still loading")
        if s["live_peers"] == 0:
            warnings.add("no live peers")
        if s["best_peer_height"] > s["tip_height"]:
            warnings.add("peer ahead of local tip")
            if s["mining"] and s["mining_idle"] != "behind a peer":
                warnings.add("mining enabled while behind with no explicit idle hold")
        if s["stalled_on_fork"] or s["reorg_in_flight"]:
            warnings.add("fork recovery active")
        if not s["supply_invariant_ok"]:
            warnings.add("node supply check not ready")
    if first["tip_height"] == second["tip_height"] and first["tip"] != second["tip"]:
        warnings.add("tip changed at same height")
    if second["tip_height"] < first["tip_height"]:
        warnings.add("tip height decreased")
    if int(second["total_work"]) < int(first["total_work"]):
        warnings.add("cumulative work decreased")
    return sorted(warnings)


def capture(port, pause):
    first = check_status(rpc(port, "status", 101))
    time.sleep(pause)
    # Chain advances during polling. Only bind supply results to a stable tip.
    # Never mistake an ordinary new block for an invalid monetary invariant.
    second = None
    for attempt in range(3):
        before = check_status(rpc(port, "status", 110 + attempt * 3))
        supply = rpc(port, "verify_supply", 111 + attempt * 3)
        after = check_status(rpc(port, "status", 112 + attempt * 3))
        if before["tip"] == after["tip"] and before["circulating"] == after["circulating"]:
            check_supply(supply, after)
            second = after
            break
    if second is None:
        raise EvidenceError("tip changed during supply check: retry")
    warnings = assess(first, second)
    body = {"schema": "nightfall-node-evidence-v1",
            "scope": "untrusted-local-rpc-claims-not-consensus-proof",
            "samples": [first, second],
            "checks": ["status", "verify_supply"],
            "assessment": "CAUTION" if warnings else "OBSERVED", "warnings": warnings}
    return seal(body)


def compare(file_a, file_b):
    a = verify_receipt(parse_json(Path(file_a).read_text(encoding="utf8")))["samples"][-1]
    b = verify_receipt(parse_json(Path(file_b).read_text(encoding="utf8")))["samples"][-1]
    if a["genesis"] != b["genesis"] or a["network"] != b["network"]:
        raise EvidenceError("network identities disagree")
    if a["tip_height"] == b["tip_height"]:
        answer = "SAME_REPORTED_TIP" if a["tip"] == b["tip"] else "CONFLICTING_REPORTED_TIPS"
    else:
        answer = "DIFFERENT_HEIGHTS_INCONCLUSIVE"
    return {"comparison": answer, "heights": [a["tip_height"], b["tip_height"]],
            "limitation": "Matching RPC claims do not prove independent validation, custody, or operators"}


def main():
    p = argparse.ArgumentParser(description="Read-only Nightfall node evidence; never a consensus proof")
    subs = p.add_subparsers(dest="command", required=True)
    c = subs.add_parser("capture")
    c.add_argument("--port", type=int, default=17881)
    c.add_argument("--pause", type=float, default=2.0)
    c.add_argument("--output", type=Path)
    d = subs.add_parser("compare")
    d.add_argument("receipt_a", type=Path)
    d.add_argument("receipt_b", type=Path)
    args = p.parse_args()
    try:
        if args.command == "capture":
            if not 1 <= args.port <= 65535 or not 0 <= args.pause <= 30:
                raise EvidenceError("port/pause outside allowed limits")
            report = capture(args.port, args.pause)
            encoded = json.dumps(report, sort_keys=True, indent=2) + "\n"
            if args.output:
                if args.output.exists():
                    raise EvidenceError("refusing to overwrite existing receipt")
                with os.fdopen(os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w", encoding="utf8") as f:
                    f.write(encoded)
                print("wrote local receipt: " + str(args.output))
            else:
                sys.stdout.write(encoded)
            return 0 if report["payload"]["assessment"] == "OBSERVED" else 1
        print(json.dumps(compare(args.receipt_a, args.receipt_b), indent=2))
        return 0
    except (EvidenceError, OSError, ValueError) as ex:
        # Never print a potentially sensitive RPC payload or filesystem contents.
        print("node evidence: " + str(ex), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
