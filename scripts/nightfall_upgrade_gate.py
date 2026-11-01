#!/usr/bin/env python3
"""Read-only, explicitly non-authoritative upgrade/readiness check for Nightfall."""

import argparse
import datetime as dt
import json
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

URL = "https://nightfallcoin.org/network.json"

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)

ACTIVATION_HEIGHT = 500_000
MIN_NODE_VERSION = (1, 1, 1)

HARD_CAP_NIGHT = 90_000_000
DARK_PER_NIGHT = 100_000_000
INITIAL_SUBSIDY_DARK = 6 * DARK_PER_NIGHT
INITIAL_ERA_BLOCKS = 7_500_000

MAX_BYTES = 256 * 1024

VERSION = re.compile(
    r"(?:^|/)(\d+)\.(\d+)\.(\d+)(?=$|[^\d.])"
)


def whole(value):
    return type(value) is int and value >= 0


def version_tuple(raw):
    if not isinstance(raw, str):
        return None

    match = VERSION.search(raw)

    if not match:
        return None

    return tuple(int(n) for n in match.groups())


def evaluate(data, now=None, max_age=1800):
    if not isinstance(data, dict):
        raise ValueError(
            "network.json must contain a JSON object"
        )

    now = int(time.time()) if now is None else int(now)

    failures = []
    warnings = []

    def expect(condition, message):
        if not condition:
            failures.append(message)

    expect(
        data.get("network") == "mainnet",
        "network is not mainnet"
    )

    expect(
        data.get("genesis") == GENESIS,
        "unexpected genesis (do not mix chain epochs)"
    )

    expect(
        data.get("protocol_version") == 8,
        "unexpected protocol version"
    )

    expect(
        data.get("wire_version") == 6,
        "unexpected wire version"
    )

    expect(
        data.get("supply_invariant_ok") is True,
        "source node did not report valid supply invariant"
    )

    expect(
        data.get("loading") is False,
        "source node reports loading or unknown status"
    )

    expect(
        data.get("max_supply") == HARD_CAP_NIGHT,
        "unexpected maximum supply"
    )

    required = (
        "blocks",
        "tip_height",
        "minted",
        "burned_fees",
        "circulating",
        "tip_time"
    )

    for name in required:
        expect(
            whole(data.get(name)),
            "missing or invalid nonnegative integer: " + name
        )

    blocks = (
        data.get("blocks")
        if whole(data.get("blocks"))
        else None
    )

    height = (
        data.get("tip_height")
        if whole(data.get("tip_height"))
        else None
    )

    minted = (
        data.get("minted")
        if whole(data.get("minted"))
        else None
    )

    burned = (
        data.get("burned_fees")
        if whole(data.get("burned_fees"))
        else None
    )

    circulating = (
        data.get("circulating")
        if whole(data.get("circulating"))
        else None
    )

    tip_time = (
        data.get("tip_time")
        if whole(data.get("tip_time"))
        else None
    )

    if blocks is not None and height is not None:
        expect(
            blocks == height + 1,
            "blocks and tip_height disagree"
        )

    if (
        minted is not None
        and burned is not None
        and circulating is not None
    ):
        expect(
            minted >= burned,
            "burned fees exceed mint total"
        )

        expect(
            minted - burned == circulating,
            "reported monetary accounting disagrees"
        )

        expect(
            minted <= HARD_CAP_NIGHT * DARK_PER_NIGHT,
            "reported mint exceeds cap"
        )

        if (
            blocks is not None
            and 0 < blocks < INITIAL_ERA_BLOCKS
        ):
            expect(
                minted == blocks * INITIAL_SUBSIDY_DARK,
                "first-era issuance differs from block count"
            )

    if tip_time is not None:
        expect(
            tip_time <= now + 120,
            "tip timestamp exceeds local clock by more than 120 seconds"
        )

        expect(
            tip_time >= now - max_age,
            "source tip is too old for upgrade readiness"
        )

    peer_versions = data.get("peer_versions")

    sample = {
        "ready": 0,
        "legacy": 0,
        "unknown": 0,
        "reported": 0
    }

    if not isinstance(peer_versions, dict):
        warnings.append(
            "peer version sample absent; upgrade adoption cannot be assessed"
        )

    else:
        for name, count in peer_versions.items():
            if not whole(count):
                failures.append(
                    "peer_versions contains invalid count"
                )
                continue

            sample["reported"] += count

            parsed = version_tuple(name)

            if parsed is None:
                sample["unknown"] += count

            elif parsed >= MIN_NODE_VERSION:
                sample["ready"] += count

            else:
                sample["legacy"] += count

        if sample["reported"] == 0:
            warnings.append(
                "peer version sample empty; upgrade adoption unknown"
            )

        if sample["legacy"]:
            warnings.append(
                "older-than-1.1.1 versions present in source node's sample"
            )

        if sample["unknown"]:
            warnings.append(
                "unrecognized versions present in source node's sample"
            )

    remaining = (
        max(0, ACTIVATION_HEIGHT - height)
        if height is not None
        else None
    )

    if remaining == 0:
        warnings.append(
            "activation height reached: upgrade remains mandatory for compatible nodes"
        )

    state = (
        "FAIL" if failures
        else "WARN" if warnings
        else "PASS"
    )

    return {
        "state": state,
        "source": URL,
        "source_is_independent_verification": False,
        "observed_utc": dt.datetime.fromtimestamp(
            now, dt.timezone.utc
        ).isoformat(),
        "tip_height": height,
        "activation_height": ACTIVATION_HEIGHT,
        "blocks_remaining": remaining,
        "source_tip_age_seconds": (
            now - tip_time
            if tip_time is not None
            else None
        ),
        "observed_peer_version_sample": sample,
        "failures": failures,
        "warnings": warnings,
        "limitations": (
            "Website node reports only; no local consensus, "
            "PoW, topology, or independent audit verification."
        )
    }


def load_json(path=None, timeout=12):
    if path is not None:
        raw = Path(path).read_bytes()

    else:
        request = urllib.request.Request(
            URL,
            headers={
                "Accept": "application/json",
                "User-Agent": "Nightfall-Upgrade-Gate/1"
            }
        )

        with urllib.request.urlopen(
            request, timeout=timeout
        ) as response:

            if response.geturl() != URL:
                raise ValueError(
                    "unexpected redirect away from pinned source URL"
                )

            raw = response.read(MAX_BYTES + 1)

    if len(raw) > MAX_BYTES:
        raise ValueError(
            "network.json exceeds size budget"
        )

    return json.loads(raw.decode("utf-8"))


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__
    )

    parser.add_argument(
        "--fixture",
        help="read local JSON instead of making an HTTPS request"
    )

    parser.add_argument(
        "--json",
        action="store_true",
        help="print machine-readable result"
    )

    parser.add_argument(
        "--max-age",
        type=int,
        default=1800,
        help="maximum source tip age in seconds"
    )

    args = parser.parse_args(argv)

    if args.max_age < 60:
        parser.error(
            "--max-age must be at least 60 seconds"
        )

    try:
        result = evaluate(
            load_json(args.fixture),
            max_age=args.max_age
        )

    except (
        OSError,
        ValueError,
        UnicodeError,
        json.JSONDecodeError,
        urllib.error.URLError
    ) as exc:

        result = {
            "state": "FAIL",
            "failures": [
                f"source unavailable or invalid: {exc}"
            ],
            "warnings": [],
            "source": URL,
            "source_is_independent_verification": False
        }

    if args.json:
        print(
            json.dumps(
                result,
                ensure_ascii=False,
                indent=2,
                sort_keys=True
            )
        )

    else:
        print(
            "Nightfall upgrade assurance:",
            result["state"]
        )

        print(
            "Source: public website node "
            "(not independently verified)"
        )

        print(
            "Height:",
            result.get("tip_height", "unknown"),
            "| until 500,000:",
            result.get("blocks_remaining", "unknown")
        )

        print(
            "Observed version sample:",
            result.get(
                "observed_peer_version_sample", {}
            )
        )

        for key in ("failures", "warnings"):
            for item in result.get(key, []):
                print(
                    key[:-1].upper() + ":",
                    item
                )

    if result["state"] == "FAIL":
        return 2

    if result["state"] == "WARN":
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
