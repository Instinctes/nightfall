#!/usr/bin/env python3
"""
Nightfall Guard.

Public network readiness and supply accounting evidence.

Read-only.
No wallet.
No private keys.
No RPC credentials.
No telemetry.
No consensus modifications.

This checker verifies public claims against the documented
integer emission schedule.

It does NOT independently verify cryptographic supply proofs.
"""

import argparse
import datetime as dt
import json
import re
import sys
import urllib.error
import urllib.request

from pathlib import Path


URL = "https://nightfallcoin.org/network.json"

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)

DARKS = 100_000_000

CAP = 90_000_000 * DARKS

ERA_LENGTH = 7_500_000

BASE_REWARD = 6 * DARKS

ACTIVATION = 500_000

MIN_VERSION = (1, 1, 1)

MAX_BODY = 32_768

HEX64 = re.compile(r"^[a-f0-9]{64}$")

VERSION = re.compile(
    r"^[a-z][a-z0-9_-]*/"
    r"(\d+)\.(\d+)\.(\d+)"
    r"(?:[-+][a-zA-Z0-9.-]+)?$"
)


def strict_uint(value, label, errors):
    """
    Require an actual non-negative JSON integer.

    Reject booleans, floats, strings and negative numbers.
    """

    if type(value) is not int or value < 0:
        errors.append(
            f"{label}: non-negative JSON integer required"
        )
        return None

    return value


def emission(blocks):
    """
    Independently calculate scheduled issuance.

    Integer arithmetic only.

    Calculates block heights in [0, blocks).

    Complexity is bounded by the number of emission eras.
    """

    if type(blocks) is not int or blocks < 0:
        raise ValueError(
            "blocks must be a non-negative integer"
        )

    remaining = blocks
    total = 0

    for era in range(BASE_REWARD.bit_length()):

        if remaining <= 0:
            break

        reward = BASE_REWARD >> era

        if reward == 0:
            break

        count = min(
            ERA_LENGTH,
            remaining
        )

        total += count * reward

        remaining -= count

    return min(total, CAP)


def evaluate(payload, previous=None, now=None):
    """
    Evaluate public reported chain facts.

    Never copy arbitrary fields from the upstream payload
    into the result.

    In particular, no addresses, keys or wallet data
    are retained.
    """

    errors = []
    warnings = []

    if now is None:
        now = int(
            dt.datetime.now(
                dt.timezone.utc
            ).timestamp()
        )

    if not isinstance(payload, dict):

        return {
            "verdict": "FAIL",
            "errors": [
                "JSON root must be an object"
            ],
            "warnings": [],
            "facts": {}
        }

    blocks = strict_uint(
        payload.get("blocks"),
        "blocks",
        errors
    )

    height = strict_uint(
        payload.get("tip_height"),
        "tip_height",
        errors
    )

    minted = strict_uint(
        payload.get("minted"),
        "minted",
        errors
    )

    burned = strict_uint(
        payload.get("burned_fees"),
        "burned_fees",
        errors
    )

    circulating = strict_uint(
        payload.get("circulating"),
        "circulating",
        errors
    )

    difficulty = strict_uint(
        payload.get("difficulty"),
        "difficulty",
        errors
    )

    peers = strict_uint(
        payload.get("peers"),
        "peers",
        errors
    )

    tip_time = strict_uint(
        payload.get("tip_time"),
        "tip_time",
        errors
    )

    pver = strict_uint(
        payload.get("protocol_version"),
        "protocol_version",
        errors
    )

    wver = strict_uint(
        payload.get("wire_version"),
        "wire_version",
        errors
    )

    total_work_raw = payload.get(
        "total_work"
    )

    # --------------------------------------------------
    # Height consistency
    # --------------------------------------------------

    if blocks is not None and height is not None:

        if blocks == 0 or height != blocks - 1:

            errors.append(
                "height/count mismatch"
            )

    # --------------------------------------------------
    # Independent emission calculation
    # --------------------------------------------------

    if blocks is not None and minted is not None:

        expected = emission(blocks)

        if minted != expected:

            errors.append(
                "minted differs from independent "
                "integer emission schedule"
            )

    # --------------------------------------------------
    # Public supply accounting
    # --------------------------------------------------

    values = (
        minted,
        burned,
        circulating
    )

    if all(v is not None for v in values):

        if burned > minted:

            errors.append(
                "burned exceeds minted"
            )

        if circulating != minted - burned:

            errors.append(
                "circulating != minted - burned"
            )

        if minted > CAP or circulating > CAP:

            errors.append(
                "90m NIGHT supply ceiling exceeded"
            )

    max_supply = payload.get(
        "max_supply"
    )

    if (
        type(max_supply) is not int
        or max_supply != 90_000_000
    ):

        errors.append(
            "max_supply must be 90000000 NIGHT"
        )

    # --------------------------------------------------
    # Chain identity
    # --------------------------------------------------

    if payload.get("network") != "mainnet":

        errors.append(
            "unexpected network"
        )

    if payload.get("genesis") != GENESIS:

        errors.append(
            "unexpected genesis"
        )

    if pver is not None and pver != 8:

        errors.append(
            "unexpected protocol version"
        )

    if wver is not None and wver != 6:

        errors.append(
            "unexpected wire version"
        )

    # --------------------------------------------------
    # Reported cryptographic invariant
    # --------------------------------------------------

    if payload.get("supply_invariant_ok") is not True:

        errors.append(
            "node did not report "
            "supply_invariant_ok=true"
        )

    # --------------------------------------------------
    # Upstream synchronization
    # --------------------------------------------------

    if payload.get("loading") is not False:

        warnings.append(
            "upstream node reports loading "
            "or unknown state"
        )

    if difficulty is not None and difficulty < 2_000:

        errors.append(
            "difficulty below documented "
            "mainnet floor"
        )

    # --------------------------------------------------
    # Hash and work encoding
    # --------------------------------------------------

    tip = payload.get("tip")

    valid_tip = (
        isinstance(tip, str)
        and HEX64.fullmatch(tip) is not None
    )

    if not valid_tip:

        errors.append(
            "tip hash must be 64 lowercase hex digits"
        )

    valid_work = (
        isinstance(total_work_raw, str)
        and re.fullmatch(
            r"[0-9]{1,64}",
            total_work_raw
        ) is not None
    )

    if not valid_work:

        errors.append(
            "total_work must be a bounded "
            "decimal string"
        )

    elif int(total_work_raw) <= 0:

        errors.append(
            "total_work must be positive"
        )

    # --------------------------------------------------
    # Block timestamp sanity
    # --------------------------------------------------

    if tip_time is not None:

        if tip_time > now + 180:

            errors.append(
                "reported tip_time is "
                "implausibly in the future"
            )

        elif now - tip_time > 1800:

            warnings.append(
                "reported last block is over "
                "30 minutes old; may be cached"
            )

    # --------------------------------------------------
    # Node version readiness
    # --------------------------------------------------

    versions = payload.get(
        "peer_versions"
    )

    old = 0
    unknown = 0
    observed = 0

    if not isinstance(versions, dict):

        warnings.append(
            "peer_versions absent or not an object"
        )

    else:

        if len(versions) > 200:

            errors.append(
                "peer_versions has too many labels"
            )

        for label, count in list(
            versions.items()
        )[:200]:

            if (
                not isinstance(label, str)
                or len(label) > 128
                or type(count) is not int
                or count < 0
            ):

                errors.append(
                    "invalid peer_versions entry"
                )

                continue

            observed += count

            match = VERSION.fullmatch(
                label
            )

            if match is None:

                unknown += count

            else:

                version = tuple(
                    int(x)
                    for x in match.groups()
                )

                if version < MIN_VERSION:

                    old += count

        if old:

            warnings.append(
                "observed version labels include "
                "pre-1.1.1 nodes"
            )

        if unknown:

            warnings.append(
                "unrecognized peer version labels"
            )

    # --------------------------------------------------
    # Consensus activation readiness
    # --------------------------------------------------

    remaining = None

    if height is not None:

        remaining = max(
            0,
            ACTIVATION - height
        )

        if (
            height < ACTIVATION
            and remaining <= 50_000
        ):

            warnings.append(
                "block 500000 consensus activation "
                "is within 50000 heights"
            )

        if height >= ACTIVATION and old:

            warnings.append(
                "pre-1.1.1 labels still reported "
                "after activation"
            )

    if peers == 0:

        warnings.append(
            "public node reports zero peers"
        )

    # --------------------------------------------------
    # Compare previous public snapshot
    # --------------------------------------------------

    if (
        isinstance(previous, dict)
        and height is not None
    ):

        ph = previous.get(
            "tip_height"
        )

        pt = previous.get(
            "tip"
        )

        pw = previous.get(
            "total_work"
        )

        if type(ph) is int and ph > height:

            warnings.append(
                "height decreased since prior sample: "
                "reorg, stale cache or node switch"
            )

        if (
            type(ph) is int
            and ph == height
            and isinstance(pt, str)
            and pt != tip
        ):

            warnings.append(
                "different tip at same height "
                "since prior sample"
            )

        if (
            isinstance(pw, str)
            and pw.isascii()
            and pw.isdecimal()
            and isinstance(total_work_raw, str)
            and total_work_raw.isascii()
            and total_work_raw.isdecimal()
            and int(pw) > int(total_work_raw)
        ):

            warnings.append(
                "reported cumulative work decreased"
            )

    # --------------------------------------------------
    # Allow-listed report
    # --------------------------------------------------

    facts = {

        "network": "mainnet",

        "tip_height": height,

        "tip": (
            tip if valid_tip else None
        ),

        "total_work": (
            total_work_raw
            if valid_work else None
        ),

        "minted_darks": minted,

        "burned_darks": burned,

        "circulating_darks": circulating,

        "expected_minted_darks": (
            emission(blocks)
            if blocks is not None
            else None
        ),

        "supply_invariant_reported_by_node": (
            payload.get("supply_invariant_ok")
            is True
        ),

        "blocks_to_activation": remaining,

        "old_version_label_observations": old,

        "unclassified_version_label_observations": (
            unknown
        ),

        "all_version_label_observations": (
            observed
        ),

        "peer_count_reported": peers,

        "note": (
            "Public endpoint claims only. "
            "Not cryptographic validation, "
            "independent audit or unique-node census."
        )
    }

    if errors:

        verdict = "FAIL"

    elif warnings:

        verdict = "WARN"

    else:

        verdict = "PASS"

    return {

        "verdict": verdict,

        "errors": errors,

        "warnings": warnings,

        "facts": facts
    }


def load_public():
    """
    Fetch only the documented public endpoint.

    Refuse redirects.
    Use TLS.
    Bound the response size.
    Apply a network timeout.
    """

    class RefuseRedirect(
        urllib.request.HTTPRedirectHandler
    ):

        def redirect_request(
            self,
            req,
            fp,
            code,
            msg,
            headers,
            newurl
        ):

            raise ValueError(
                "Refusing redirect away from "
                "the fixed public endpoint"
            )

    opener = urllib.request.build_opener(
        RefuseRedirect()
    )

    req = urllib.request.Request(

        URL,

        headers={

            "Accept": "application/json",

            "User-Agent": (
                "nightfall-public-readiness/1.0"
            )
        }
    )

    with opener.open(
        req,
        timeout=12
    ) as response:

        if response.status != 200:

            raise ValueError(
                "public endpoint did not return HTTP 200"
            )

        data = response.read(
            MAX_BODY + 1
        )

        if len(data) > MAX_BODY:

            raise ValueError(
                "public JSON exceeds 32 KiB"
            )

    return json.loads(
        data.decode("utf-8")
    )


def read_json(path):

    path = Path(path)

    if path.stat().st_size > MAX_BODY:

        raise ValueError(
            "file exceeds 32 KiB"
        )

    return json.loads(
        path.read_text(
            encoding="utf-8"
        )
    )


def atomic_json(path, value):
    """
    Atomically replace local JSON evidence.
    """

    path = Path(path).expanduser()

    path.parent.mkdir(
        parents=True,
        exist_ok=True
    )

    tmp = path.with_name(
        path.name + ".tmp"
    )

    tmp.write_text(

        json.dumps(
            value,
            ensure_ascii=False,
            indent=2,
            sort_keys=True
        ) + "\n",

        encoding="utf-8"
    )

    tmp.replace(path)


def main(argv=None):

    parser = argparse.ArgumentParser(
        description=__doc__
    )

    parser.add_argument(

        "--input",

        help=(
            "Use local network.json "
            "instead of public endpoint"
        )
    )

    parser.add_argument(

        "--state",

        help=(
            "Previous public snapshot "
            "and current snapshot output"
        )
    )

    parser.add_argument(

        "--report",

        help=(
            "Report JSON output"
        )
    )

    args = parser.parse_args(
        argv
    )

    try:

        if args.input:

            payload = read_json(
                args.input
            )

        else:

            payload = load_public()

        previous = None

        if (
            args.state
            and Path(args.state).exists()
        ):

            previous = read_json(
                args.state
            )

        report = evaluate(
            payload,
            previous
        )

        report["sampled_at_utc"] = (
            dt.datetime.now(
                dt.timezone.utc
            ).isoformat(
                timespec="seconds"
            )
        )

        if args.report:

            atomic_json(
                args.report,
                report
            )

        # No raw upstream payload is persisted.
        # Only public chain state is retained.

        if (
            args.state
            and report["verdict"] != "FAIL"
        ):

            keys = (
                "tip_height",
                "tip",
                "total_work"
            )

            state = {

                k: report["facts"][k]

                for k in keys
            }

            atomic_json(
                args.state,
                state
            )

        print(
            "NIGHTFALL PUBLIC READINESS:",
            report["verdict"]
        )

        fields = (

            "tip_height",

            "blocks_to_activation",

            "minted_darks",

            "expected_minted_darks",

            "burned_darks",

            "circulating_darks",

            "old_version_label_observations"
        )

        for field in fields:

            value = report[
                "facts"
            ].get(field)

            print(
                f"  {field}: {value}"
            )

        for message in report["errors"]:

            print(
                "  FAIL:",
                message
            )

        for message in report["warnings"]:

            print(
                "  WARN:",
                message
            )

        print(
            "  LIMIT: A single remote node's claims "
            "are NOT an independent on-chain supply proof."
        )

        if report["verdict"] == "FAIL":

            return 2

        return 0

    except (
        OSError,
        ValueError,
        UnicodeError,
        json.JSONDecodeError,
        urllib.error.URLError
    ) as exc:

        print(

            "NIGHTFALL READINESS: "
            "unable to verify endpoint:",

            exc,

            file=sys.stderr
        )

        return 3


if __name__ == "__main__":

    sys.exit(
        main()
    )
