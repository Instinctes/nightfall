#!/usr/bin/env python3
"""
Nightfall Node Continuity Guard.

Read-only longitudinal verification of local
Nightfall node evidence.

Never modifies:
- blockchain databases
- wallet files
- miner seeds
- mining configuration
- node processes
- network configuration

This is not an independent consensus verifier.
"""

import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import stat
import sys

SCHEMA = "nightfall-continuity-anchor-v1"

LIMIT = 4096

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)


class GuardError(Exception):
    pass


def canonical(obj):
    return json.dumps(
        obj,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        allow_nan=False
    ).encode("ascii")


def no_duplicates(pairs):
    output = {}

    for key, value in pairs:
        if key in output:
            raise GuardError(
                "duplicate JSON key"
            )

        output[key] = value

    return output


def evidence():
    path = Path(__file__).with_name(
        "node-evidence.py"
    )

    spec = importlib.util.spec_from_file_location(
        "nightfall_node_evidence",
        path
    )

    if spec is None or spec.loader is None:
        raise GuardError(
            "node-evidence.py is required"
        )

    module = importlib.util.module_from_spec(
        spec
    )

    spec.loader.exec_module(module)

    return module


def reference(sample):
    return {
        "genesis": sample["genesis"],
        "network": sample["network"],
        "protocol_version": sample[
            "protocol_version"
        ],
        "wire_version": sample[
            "wire_version"
        ],
        "height": sample["tip_height"],
        "tip": sample["tip"],
        "total_work": str(
            int(sample["total_work"])
        )
    }


def validate_anchor(a):
    required = {
        "genesis",
        "network",
        "protocol_version",
        "wire_version",
        "height",
        "tip",
        "total_work"
    }

    if not isinstance(a, dict):
        raise GuardError(
            "invalid anchor"
        )

    if set(a) != required:
        raise GuardError(
            "invalid anchor fields"
        )

    if a["network"] != "mainnet":
        raise GuardError(
            "invalid network"
        )

    if a["genesis"] != GENESIS:
        raise GuardError(
            "invalid genesis"
        )

    if (
        type(a["protocol_version"]) is not int
        or a["protocol_version"] != 8
    ):
        raise GuardError(
            "invalid protocol version"
        )

    if (
        type(a["wire_version"]) is not int
        or a["wire_version"] != 6
    ):
        raise GuardError(
            "invalid wire version"
        )

    if (
        type(a["height"]) is not int
        or a["height"] < 0
        or a["height"] >= 2**64
    ):
        raise GuardError(
            "invalid block height"
        )

    tip = a["tip"]

    if (
        type(tip) is not str
        or len(tip) != 64
        or any(
            c not in "0123456789abcdef"
            for c in tip
        )
    ):
        raise GuardError(
            "invalid block hash"
        )

    work = a["total_work"]

    if (
        type(work) is not str
        or not work.isascii()
        or not work.isdigit()
        or len(work) > 39
        or int(work) <= 0
        or str(int(work)) != work
    ):
        raise GuardError(
            "invalid cumulative work"
        )

    return a


def read_file(path):
    try:
        info = path.lstat()

    except FileNotFoundError:
        return None

    if (
        not stat.S_ISREG(info.st_mode)
        or info.st_size > LIMIT
    ):
        raise GuardError(
            "anchor must be a small regular file"
        )

    flags = (
        os.O_RDONLY
        | getattr(os, "O_NOFOLLOW", 0)
    )

    with os.fdopen(
        os.open(path, flags),
        "rb"
    ) as stream:

        data = stream.read(
            LIMIT + 1
        )

    if len(data) > LIMIT:
        raise GuardError(
            "oversized anchor"
        )

    try:
        return json.loads(
            data.decode("utf-8"),
            object_pairs_hook=no_duplicates,
            parse_constant=lambda _: (
                _ for _ in ()
            ).throw(
                GuardError("nonfinite JSON")
            )
        )

    except (
        ValueError,
        UnicodeError,
        RecursionError,
        TypeError
    ) as exc:

        raise GuardError(
            "malformed anchor JSON"
        ) from exc


def load_anchor(path):
    obj = read_file(path)

    if obj is None:
        return None

    if (
        type(obj) is not dict
        or set(obj) != {
            "schema",
            "anchor",
            "sha256"
        }
        or obj["schema"] != SCHEMA
        or type(obj["sha256"]) is not str
    ):
        raise GuardError(
            "invalid anchor container"
        )

    anchor = validate_anchor(
        obj["anchor"]
    )

    expected = hashlib.sha256(
        canonical({
            "schema": SCHEMA,
            "anchor": anchor
        })
    ).hexdigest()

    if obj["sha256"] != expected:
        raise GuardError(
            "anchor integrity mismatch"
        )

    return anchor


def atomic_save(path, anchor):
    if path.is_symlink():
        raise GuardError(
            "refusing symlink destination"
        )

    body = {
        "schema": SCHEMA,
        "anchor": validate_anchor(
            anchor
        )
    }

    body["sha256"] = hashlib.sha256(
        canonical(body)
    ).hexdigest()

    tmp = path.with_name(
        "."
        + path.name
        + "."
        + secrets.token_hex(12)
        + ".tmp"
    )

    try:
        flags = (
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(
                os,
                "O_NOFOLLOW",
                0
            )
        )

        fd = os.open(
            tmp,
            flags,
            0o600
        )

        with os.fdopen(
            fd,
            "wb"
        ) as stream:

            stream.write(
                canonical(body) + b"\n"
            )

            stream.flush()

            os.fsync(
                stream.fileno()
            )

        os.replace(
            tmp,
            path
        )

        directory_fd = os.open(
            path.parent,
            os.O_RDONLY
        )

        try:
            os.fsync(
                directory_fd
            )

        finally:
            os.close(
                directory_fd
            )

    finally:
        if tmp.exists():
            tmp.unlink()


def assess(
    old,
    current,
    receipt_warnings
):
    alerts = [
        "node-evidence: " + x
        for x in receipt_warnings
    ]

    if old is None:
        return sorted(alerts)

    for key in (
        "network",
        "genesis",
        "protocol_version",
        "wire_version"
    ):
        if old[key] != current[key]:
            alerts.append(
                "network identity changed"
            )
            break

    old_work = int(
        old["total_work"]
    )

    new_work = int(
        current["total_work"]
    )

    if (
        current["height"]
        < old["height"]
    ):
        alerts.append(
            "height decreased: possible reorg or rollback"
        )

    if new_work < old_work:
        alerts.append(
            "cumulative work decreased"
        )

    if (
        current["height"] > old["height"]
        and new_work <= old_work
    ):
        alerts.append(
            "height rose without higher cumulative work"
        )

    if (
        current["height"] == old["height"]
        and current["tip"] != old["tip"]
    ):
        alerts.append(
            "different tip at same height"
        )

    if (
        current["height"] == old["height"]
        and current["tip"] == old["tip"]
        and new_work != old_work
    ):
        alerts.append(
            "same tip reports different cumulative work"
        )

    if (
        current["height"] > old["height"]
        and current["tip"] == old["tip"]
    ):
        alerts.append(
            "same tip at different height"
        )

    return sorted(
        set(alerts)
    )


def evaluate(
    body,
    path,
    *,
    dry_run=False
):
    if (
        type(body) is not dict
        or body.get("schema")
        != "nightfall-node-evidence-v1"
    ):
        raise GuardError(
            "unverified or incompatible evidence"
        )

    latest = reference(
        body["samples"][-1]
    )

    path.parent.mkdir(
        parents=True,
        exist_ok=True,
        mode=0o700
    )

    if path.parent.is_symlink():
        raise GuardError(
            "anchor directory is a symlink"
        )

    lock_path = path.with_name(
        path.name + ".lock"
    )

    flags = (
        os.O_RDWR
        | os.O_CREAT
        | getattr(
            os,
            "O_NOFOLLOW",
            0
        )
    )

    with os.fdopen(
        os.open(
            lock_path,
            flags,
            0o600
        ),
        "rb+"
    ) as lock:

        try:
            fcntl.flock(
                lock.fileno(),
                fcntl.LOCK_EX
                | fcntl.LOCK_NB
            )

        except BlockingIOError as exc:
            raise GuardError(
                "another continuity check is running"
            ) from exc

        old = load_anchor(
            path
        )

        alerts = assess(
            old,
            latest,
            body["warnings"]
        )

        changed = False

        if alerts:
            status = "CAUTION"

        elif old is None:
            status = "BASELINED"
            changed = True

        elif old == latest:
            status = "UNCHANGED"

        else:
            status = "ADVANCED"
            changed = True

        if changed and not dry_run:
            atomic_save(
                path,
                latest
            )

        return {
            "schema":
                "nightfall-continuity-result-v1",
            "status": status,
            "alerts": alerts,
            "previous_height":
                old["height"]
                if old else None,
            "observed_height":
                latest["height"],
            "previous_work":
                old["total_work"]
                if old else None,
            "observed_work":
                latest["total_work"],
            "state_written":
                bool(
                    changed
                    and not dry_run
                ),
            "limitation":
                "Local RPC claims only; not a consensus proof or authenticated history"
        }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__
    )

    parser.add_argument(
        "--state",
        type=Path,
        default=(
            Path.home()
            / ".nightfall-continuity"
            / "anchor.json"
        )
    )

    parser.add_argument(
        "--receipt",
        type=Path,
        help="Existing offline node evidence receipt"
    )

    parser.add_argument(
        "--port",
        type=int,
        default=17881
    )

    parser.add_argument(
        "--pause",
        type=float,
        default=2.0
    )

    parser.add_argument(
        "--dry-run",
        action="store_true"
    )

    args = parser.parse_args(
        argv
    )

    try:
        if not (
            1 <= args.port <= 65535
            and 0 <= args.pause <= 30
        ):
            raise GuardError(
                "port or pause out of range"
            )

        kit = evidence()

        try:
            if args.receipt:
                body = kit.read_receipt(
                    args.receipt
                )

            else:
                body = kit.verify_receipt(
                    kit.capture(
                        args.port,
                        args.pause
                    )
                )

        except kit.EvidenceError as exc:
            raise GuardError(
                str(exc)
            ) from exc

        result = evaluate(
            body,
            args.state.expanduser(),
            dry_run=args.dry_run
        )

        print(
            json.dumps(
                result,
                indent=2,
                sort_keys=True
            )
        )

        if result["status"] == "CAUTION":
            return 1

        return 0

    except (
        GuardError,
        OSError,
        ValueError,
        KeyError,
        TypeError
    ) as exc:

        print(
            "continuity guard: " + str(exc),
            file=sys.stderr
        )

        return 2


if __name__ == "__main__":
    sys.exit(main())
