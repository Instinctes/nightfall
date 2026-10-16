#!/usr/bin/env python3
"""Challenge-bound Ed25519 attestations over Nightfall node-evidence receipts.

Authenticates a pinned key's statement about local RPC claims, NOT chain truth.
No network, wallet, mining, or consensus operations.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("nightfall_node_evidence", HERE / "node-evidence.py")
node = importlib.util.module_from_spec(spec)
spec.loader.exec_module(node)

DOMAIN = b"NIGHTFALL-OPERATOR-WITNESS-v1\x00"
SCHEMA = "nightfall-operator-witness-v1"
TRUST_SCHEMA = "nightfall-operator-trust-v1"
ED25519_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
MAX_FILE = 131072


class WitnessError(Exception):
    pass


def read_json(path):
    try:
        with open(path, "rb") as f:
            content = f.read(MAX_FILE + 1)
        if len(content) > MAX_FILE:
            raise WitnessError("input exceeds size limit")
        return node.parse_json(content.decode("utf-8", "strict"))
    except (OSError, UnicodeError) as ex:
        raise WitnessError("cannot read input") from ex


def b64_decode(value, limit):
    if not isinstance(value, str) or len(value) > 1024:
        raise WitnessError("invalid base64 field")
    try:
        result = base64.b64decode(value, validate=True)
    except (ValueError, base64.binascii.Error) as ex:
        raise WitnessError("invalid base64 field") from ex
    if len(result) > limit:
        raise WitnessError("oversized binary field")
    return result


def check_pubkey(der):
    if len(der) != 44 or not der.startswith(ED25519_SPKI_PREFIX):
        raise WitnessError("key must be canonical Ed25519 SPKI DER")
    return hashlib.sha256(der).hexdigest()


def hex_challenge(s):
    if not isinstance(s, str) or len(s) != 64 or any(c not in "0123456789abcdef" for c in s):
        raise WitnessError("expected 32-byte lowercase hex challenge")
    return s


def checked_body(body):
    if not isinstance(body, dict) or set(body) != {"schema", "operator", "receipt", "challenge"}:
        raise WitnessError("unexpected signed body fields")
    hex_challenge(body["challenge"])
    if body["schema"] != SCHEMA:
        raise WitnessError("unsupported signature domain")
    if not isinstance(body["operator"], str) or len(body["operator"]) != 64 or any(c not in "0123456789abcdef" for c in body["operator"]):
        raise WitnessError("invalid operator fingerprint")
    try:
        evidence = node.verify_receipt(body["receipt"])
    except node.EvidenceError as ex:
        raise WitnessError("invalid underlying node evidence") from ex
    return evidence["samples"][-1], evidence


def message(body):
    checked_body(body)
    return DOMAIN + node.canonical(body)


def openssl(*args):
    try:
        run = subprocess.run(["openssl", *map(str, args)], capture_output=True,
                             timeout=10, check=False)
    except (OSError, subprocess.TimeoutExpired) as ex:
        raise WitnessError("OpenSSL unavailable or timed out") from ex
    if run.returncode:
        raise WitnessError("OpenSSL key/signature operation failed")
    return run.stdout


def sign(receipt, private_key, challenge):
    try:
        info = private_key.lstat()
    except OSError as ex:
        raise WitnessError("private key inaccessible") from ex
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        raise WitnessError("private key must be a regular file with chmod 600")
    pk = openssl("pkey", "-in", private_key, "-pubout", "-outform", "DER")
    fingerprint = check_pubkey(pk)
    body = {"schema": SCHEMA, "operator": fingerprint, "receipt": receipt,
            "challenge": hex_challenge(challenge)}
    msg = message(body)
    with tempfile.TemporaryDirectory(prefix="nightfall-witness-") as tmp:
        m = Path(tmp) / "message"
        sig = Path(tmp) / "signature"
        m.write_bytes(msg)
        openssl("pkeyutl", "-sign", "-inkey", private_key,
                "-rawin", "-in", m, "-out", sig)
        signature = sig.read_bytes()
    if len(signature) != 64:
        raise WitnessError("invalid Ed25519 signature length")
    return {"body": body, "public_key_spki_b64": base64.b64encode(pk).decode("ascii"),
            "signature_b64": base64.b64encode(signature).decode("ascii")}


def verify(attestation, allowed, challenge):
    if not isinstance(attestation, dict) or set(attestation) != {"body", "public_key_spki_b64", "signature_b64"}:
        raise WitnessError("invalid attestation container")
    body = attestation["body"]
    status, evidence = checked_body(body)
    if body["challenge"] != hex_challenge(challenge):
        raise WitnessError("challenge mismatch/replay")
    pk = b64_decode(attestation["public_key_spki_b64"], 44)
    fingerprint = check_pubkey(pk)
    if body["operator"] != fingerprint or fingerprint not in allowed:
        raise WitnessError("untrusted or substituted operator key")
    signature = b64_decode(attestation["signature_b64"], 64)
    if len(signature) != 64:
        raise WitnessError("invalid signature size")
    with tempfile.TemporaryDirectory(prefix="nightfall-witness-") as tmp:
        pub = Path(tmp) / "key.der"
        msg = Path(tmp) / "message"
        sig = Path(tmp) / "signature"
        pub.write_bytes(pk)
        msg.write_bytes(message(body))
        sig.write_bytes(signature)
        openssl("pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", pub,
                "-rawin", "-in", msg, "-sigfile", sig)
    return fingerprint, status, evidence


def trust_file(path):
    p = read_json(path)
    if not isinstance(p, dict) or set(p) != {"schema", "fingerprints"} or p["schema"] != TRUST_SCHEMA:
        raise WitnessError("invalid trust manifest")
    a = p["fingerprints"]
    if not isinstance(a, list) or len(a) < 2 or len(a) > 32 or any(
        not isinstance(x, str) or len(x) != 64 or any(c not in "0123456789abcdef" for c in x) for x in a
    ) or len(set(a)) != len(a):
        raise WitnessError("pin at least two unique Ed25519 fingerprints")
    return set(a)


def reconcile(attestations, allowed, challenge):
    if not 2 <= len(attestations) <= 32:
        raise WitnessError("need 2..32 signed observations")
    seen = set()
    statuses = []
    assessments = []
    for document in attestations:
        fingerprint, status, evidence = verify(document, allowed, challenge)
        if fingerprint in seen:
            raise WitnessError("duplicate operator key cannot increase witness count")
        seen.add(fingerprint)
        statuses.append(status)
        assessments.append(evidence["assessment"])
    heights = {s["tip_height"] for s in statuses}
    if len(heights) != 1:
        decision = "DIFFERENT_HEIGHTS_INCONCLUSIVE"
    elif len({(s["tip"], s["total_work"], s["minted"], s["burned_fees"], s["circulating"]) for s in statuses}) != 1:
        decision = "CONFLICTING_REPORTED_STATE"
    elif any(a != "OBSERVED" for a in assessments):
        decision = "AGREE_BUT_LOCAL_CAUTION"
    else:
        decision = "CONSISTENT_REPORTED_STATE"
    return {"schema": "nightfall-witness-reconciliation-v1", "result": decision,
            "signed_distinct_keys": len(seen), "heights": sorted(heights),
            "scope": "operator-authenticated-RPC-claims-not-chain-validation-or-independent-ownership"}


def exclusive_write(path, value):
    data = (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode("ascii")
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
    except OSError as ex:
        raise WitnessError("output exists or cannot be created") from ex


def main(argv=None):
    parser = argparse.ArgumentParser(description="Authenticates node-evidence receipts; does not audit consensus")
    subs = parser.add_subparsers(dest="command", required=True)
    s = subs.add_parser("sign")
    s.add_argument("--receipt", type=Path, required=True)
    s.add_argument("--challenge", required=True)
    s.add_argument("--private-key", type=Path, required=True)
    s.add_argument("--output", type=Path, required=True)
    v = subs.add_parser("verify")
    v.add_argument("--trust", type=Path, required=True)
    v.add_argument("--challenge", required=True)
    v.add_argument("attestations", nargs="+", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "sign":
            doc = read_json(args.receipt)
            node.verify_receipt(doc)
            exclusive_write(args.output, sign(doc, args.private_key, args.challenge))
            print("Signed redacted RPC observation (not a consensus proof).")
        else:
            result = reconcile([read_json(p) for p in args.attestations], trust_file(args.trust), args.challenge)
            print(json.dumps(result, indent=2))
            return 0 if result["result"] == "CONSISTENT_REPORTED_STATE" else 1
    except (WitnessError, node.EvidenceError) as ex:
        print("operator witness: " + str(ex), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
