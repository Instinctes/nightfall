#!/usr/bin/env python3
"""Untrusted public network observations, never a consensus or supply proof."""

import argparse
import hashlib
import json
import re
import sys
import time
import urllib.request

from pathlib import Path

URL = "https://nightfallcoin.org/network.json"

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)

MAX_BYTES = 65536
MAX_DARKS = 90_000_000 * 100_000_000

FIELDS = (
    "network",
    "genesis",
    "protocol_version",
    "wire_version",
    "blocks",
    "tip_height",
    "tip",
    "tip_time",
    "difficulty",
    "total_work",
    "minted",
    "burned_fees",
    "circulating",
    "max_supply",
    "supply_invariant_ok",
    "loading",
    "peers",
    "peer_versions",
)

VERSION = re.compile(
    r"nightfalld/[0-9]+"
    r"(?:\.[0-9]+){1,3}"
    r"(?:[+._-][a-z0-9._-]{1,20})?\Z"
)

HEX = re.compile(r"[0-9a-f]{64}\Z")


class InvalidPublicData(ValueError):
    pass


def unique_pairs(pairs):
    out = {}

    for k, v in pairs:
        if k in out:
            raise InvalidPublicData(
                "duplicate JSON object key"
            )

        out[k] = v

    return out


def read_json(data):
    if len(data) > MAX_BYTES:
        raise InvalidPublicData(
            "response exceeds 64 KiB"
        )

    try:
        return json.loads(
            data.decode("utf-8", "strict"),
            object_pairs_hook=unique_pairs,
            parse_constant=lambda _: (
                _ for _ in ()
            ).throw(
                InvalidPublicData(
                    "nonfinite number"
                )
            ),
            parse_int=lambda n: (
                int(n)
                if len(n.lstrip("-")) <= 39
                else (
                    _ for _ in ()
                ).throw(
                    InvalidPublicData(
                        "oversized integer"
                    )
                )
            ),
        )

    except (
        UnicodeError,
        TypeError,
        RecursionError,
        json.JSONDecodeError,
        ValueError,
    ) as e:
        raise InvalidPublicData(
            "invalid JSON: " + str(e)
        ) from e


def unsigned(
    value,
    name,
    maximum=(1 << 128) - 1,
):
    if (
        type(value) is not int
        or not 0 <= value <= maximum
    ):
        raise InvalidPublicData(
            "invalid unsigned field: " + name
        )

    return value


def clean(raw):
    if not isinstance(raw, dict):
        raise InvalidPublicData(
            "response must be an object"
        )

    missing = set(FIELDS) - set(raw)

    if missing:
        raise InvalidPublicData(
            "missing required fields: "
            + ", ".join(sorted(missing))
        )

    if (
        raw["network"] != "mainnet"
        or raw["genesis"] != GENESIS
    ):
        raise InvalidPublicData(
            "wrong network/genesis"
        )

    if (
        unsigned(
            raw["protocol_version"],
            "protocol_version",
        ) != 8
        or unsigned(
            raw["wire_version"],
            "wire_version",
        ) != 6
    ):
        raise InvalidPublicData(
            "protocol identity mismatch"
        )

    if (
        not isinstance(raw["tip"], str)
        or HEX.fullmatch(raw["tip"]) is None
    ):
        raise InvalidPublicData(
            "bad tip hash"
        )

    for k in (
        "blocks",
        "tip_height",
        "tip_time",
        "difficulty",
        "minted",
        "burned_fees",
        "circulating",
        "max_supply",
        "peers",
    ):
        unsigned(raw[k], k)

    if (
        raw["blocks"]
        != raw["tip_height"] + 1
    ):
        raise InvalidPublicData(
            "blocks and tip_height disagree"
        )

    if raw["max_supply"] != 90_000_000:
        raise InvalidPublicData(
            "unexpected supply cap"
        )

    if raw["difficulty"] < 2000:
        raise InvalidPublicData(
            "difficulty below mainnet floor"
        )

    if (
        raw["minted"] > MAX_DARKS
        or raw["burned_fees"] > raw["minted"]
    ):
        raise InvalidPublicData(
            "monetary bounds violated"
        )

    if (
        raw["circulating"]
        != raw["minted"] - raw["burned_fees"]
    ):
        raise InvalidPublicData(
            "reported monetary accounting inconsistent"
        )

    work = raw["total_work"]

    if (
        type(work) is not str
        or not 1 <= len(work) <= 39
        or not work.isascii()
        or not work.isdecimal()
        or work[0] == "0"
        or int(work) > (1 << 128) - 1
    ):
        raise InvalidPublicData(
            "bad cumulative work"
        )

    if (
        type(raw["supply_invariant_ok"])
        is not bool
        or type(raw["loading"])
        is not bool
    ):
        raise InvalidPublicData(
            "invalid boolean"
        )

    versions = raw["peer_versions"]

    if (
        not isinstance(versions, dict)
        or len(versions) > 64
    ):
        raise InvalidPublicData(
            "invalid version aggregates"
        )

    for version, count in versions.items():
        if (
            type(version) is not str
            or VERSION.fullmatch(version) is None
        ):
            raise InvalidPublicData(
                "unrecognized or private version label"
            )

        unsigned(
            count,
            "peer_versions count",
            1_000_000,
        )

    # Explicit privacy allowlist.
    # Never export unknown source fields.

    out = {
        key: raw[key]
        for key in FIELDS
    }

    out["peer_versions"] = dict(
        sorted(versions.items())
    )

    return out


def evaluate(samples, now):
    findings = set()

    for entry in samples:
        s = entry["public"]

        if s["supply_invariant_ok"] is False:
            findings.add(
                "REPORTED_SUPPLY_CHECK_FALSE"
            )

        if s["loading"]:
            findings.add(
                "NODE_LOADING"
            )

        if s["peers"] == 0:
            findings.add(
                "NO_REPORTED_PEERS"
            )

        if s["tip_time"] > now + 120:
            findings.add(
                "TIP_TIMESTAMP_IN_FUTURE"
            )

        if now > s["tip_time"] + 900:
            findings.add(
                "TIP_OLDER_THAN_15_MINUTES"
            )

    for a, b in zip(
        samples,
        samples[1:],
    ):
        x = a["public"]
        y = b["public"]

        if (
            int(y["total_work"])
            < int(x["total_work"])
        ):
            findings.add(
                "REPORTED_WORK_REGRESSION"
            )

        if (
            y["tip_height"]
            < x["tip_height"]
        ):
            findings.add(
                "REPORTED_HEIGHT_ROLLBACK_CHECK_REORG"
            )

        if (
            x["tip_height"]
            == y["tip_height"]
            and x["tip"] != y["tip"]
        ):
            findings.add(
                "REPORTED_SAME_HEIGHT_TIP_CHANGE"
            )

        if (
            x["tip"] == y["tip"]
            and x["tip_height"]
            != y["tip_height"]
        ):
            findings.add(
                "INCONSISTENT_TIP_HEIGHT_PAIR"
            )

    return sorted(findings)


def digest(value):
    payload = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
        allow_nan=False,
    ).encode("ascii")

    return hashlib.sha256(
        payload
    ).hexdigest()


def build_report(samples, now):
    findings = evaluate(
        samples,
        now,
    )

    report = {
        "schema": "nightfall-public-network-health-v1",
        "scope": (
            "public-node-claims-not-independent-"
            "consensus-verification"
        ),
        "source": URL,
        "samples": samples,
        "findings": findings,
        "assessment": (
            "CAUTION"
            if findings
            else "OBSERVED"
        ),
    }

    report["sha256"] = digest(report)

    return report


class NoRedirect(
    urllib.request.HTTPRedirectHandler
):
    def redirect_request(
        self,
        req,
        fp,
        code,
        msg,
        headers,
        newurl,
    ):
        raise InvalidPublicData(
            "HTTP redirect forbidden"
        )


def fetch():
    request = urllib.request.Request(
        URL,
        headers={
            "User-Agent": "NightfallPublicHealth/1.0",
            "Accept": "application/json",
        },
    )

    opener = urllib.request.build_opener(
        NoRedirect()
    )

    with opener.open(
        request,
        timeout=12,
    ) as response:

        if response.geturl() != URL:
            raise InvalidPublicData(
                "HTTP redirect not allowed"
            )

        if response.status != 200:
            raise InvalidPublicData(
                "non-200 status"
            )

        return read_json(
            response.read(MAX_BYTES + 1)
        )


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=(
            "Privacy-safe public network "
            "status evidence; not a consensus proof"
        )
    )

    parser.add_argument(
        "--samples",
        type=int,
        default=2,
        choices=(1, 2),
    )

    parser.add_argument(
        "--interval",
        type=int,
        default=3,
    )

    parser.add_argument(
        "--output",
        type=Path,
    )

    args = parser.parse_args(argv)

    if not 0 <= args.interval <= 60:
        parser.error(
            "--interval must be 0..60 seconds"
        )

    samples = []

    try:
        for n in range(args.samples):
            sample = clean(fetch())

            samples.append({
                "observed_unix": int(time.time()),
                "public": sample,
            })

            if n + 1 < args.samples:
                time.sleep(
                    args.interval
                )

        report = build_report(
            samples,
            int(time.time()),
        )

        rendered = json.dumps(
            report,
            indent=2,
            sort_keys=True,
            ensure_ascii=True,
        ) + "\n"

        if args.output:
            args.output.write_text(
                rendered,
                encoding="utf-8",
            )
        else:
            print(
                rendered,
                end="",
            )

        print(
            "PUBLIC HEALTH: "
            + report["assessment"]
            + "; findings="
            + ",".join(report["findings"]),
            file=sys.stderr,
        )

        return 0

    except (
        InvalidPublicData,
        OSError,
        TimeoutError,
    ) as e:

        print(
            "PUBLIC HEALTH: unavailable/invalid "
            "public report: " + str(e),
            file=sys.stderr,
        )

        return 2


if __name__ == "__main__":
    sys.exit(main())
