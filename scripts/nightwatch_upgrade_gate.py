#!/usr/bin/env python3
"""Post-activation network upgrade evidence."""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import sys

import nightwatch as nw

ACTIVATION_HEIGHT = 500000
MIN_VERSION = (1, 1, 1)

VERSION_LABEL = re.compile(
    r"nightfalld/(\d+)\.(\d+)\.(\d+)\Z"
)


def classify(labels):
    data = {
        "legacy": 0,
        "compatible": 0,
        "unclassified": 0,
    }

    versions = {}

    if not isinstance(labels, dict):
        raise ValueError("invalid peer_versions")

    for label, count in labels.items():

        if type(count) is not int or count < 0:
            raise ValueError("invalid version count")

        match = VERSION_LABEL.fullmatch(label)

        if match is None:
            category = "unclassified"

        else:
            version = tuple(
                int(p) for p in match.groups()
            )

            if version < MIN_VERSION:
                category = "legacy"

            elif version in (
                (1, 1, 1),
                (1, 1, 2),
            ):
                category = "compatible"

            else:
                category = "unclassified"

        data[category] += count

        versions[label] = {
            "count": count,
            "category": category,
        }

    data["versions"] = dict(
        sorted(versions.items())
    )

    data["total_reported_labels"] = sum(
        data[k]
        for k in (
            "legacy",
            "compatible",
            "unclassified",
        )
    )

    return data


def as_utc(raw):
    t = datetime.fromisoformat(
        raw.replace("Z", "+00:00")
    )

    if t.tzinfo is None:
        raise ValueError(
            "observation timestamp lacks timezone"
        )

    return t.astimezone(timezone.utc)


def evaluate(rows):

    nw.validate_history(rows)

    if not rows:
        raise ValueError(
            "no observations recorded"
        )

    tail = rows[-144:]
    samples = []

    for row in tail:

        claim = row["claim"]

        ts = as_utc(
            row["time_utc"]
        )

        versions = classify(
            claim["peer_versions"]
        )

        samples.append({
            "time_utc": ts.isoformat(
                timespec="seconds"
            ),
            "height": claim["tip_height"],
            "tip": claim["tip"],
            "legacy": versions["legacy"],
            "compatible": versions["compatible"],
        })

    latest = samples[-1]

    current = classify(
        rows[-1]["claim"]["peer_versions"]
    )

    post_activation = (
        latest["height"] >= ACTIVATION_HEIGHT
    )

    older = [
        s for s in samples
        if s["legacy"] > 0
        and s["height"] >= ACTIVATION_HEIGHT
    ]

    persistent = (
        len(older) >= 2
        and (
            as_utc(older[-1]["time_utc"])
            - as_utc(older[0]["time_utc"])
        ).total_seconds() >= 600
    )

    if not post_activation:
        verdict = "PRE_ACTIVATION_SAMPLE"

    elif current["legacy"] > 0:
        verdict = "LEGACY_LABELS_OBSERVED"

    else:
        verdict = (
            "NO_LEGACY_LABELS_IN_LAST_SAMPLE"
        )

    caution = []

    if current["legacy"] and post_activation:

        caution.append(
            "Reported pre-1.1.1 versions cannot "
            "follow post-500000 consensus rules."
        )

    if persistent:

        caution.append(
            "Legacy labels observed at least "
            "10 minutes apart on the same source."
        )

    reported_peers = (
        rows[-1]["claim"]["peers"]
    )

    if (
        current["total_reported_labels"]
        != reported_peers
    ):
        caution.append(
            "Version-label total differs from "
            "reported peer count. "
            "Do not equate these measurements."
        )

    if len(samples) >= 2:

        first = samples[0]

        span = (
            as_utc(latest["time_utc"])
            - as_utc(first["time_utc"])
        ).total_seconds()

        same_tip = all(
            s["tip"] == first["tip"]
            for s in samples
        )

        if span >= 900 and same_tip:

            caution.append(
                "One source reported an unchanged "
                "tip over at least 15 minutes. "
                "Independent verification needed."
            )

    return {
        "schema": "nightwatch-upgrade-evidence-v1",
        "latest_sample_utc": latest["time_utc"],
        "observations_considered": len(samples),
        "activation_height": ACTIVATION_HEIGHT,
        "minimum_version": "1.1.1",
        "latest_height": latest["height"],
        "status": verdict,
        "persistent_legacy_labels": persistent,
        "reported_legacy_labels": current["legacy"],
        "reported_compatible_labels": (
            current["compatible"]
        ),
        "reported_unclassified_labels": (
            current["unclassified"]
        ),
        "reported_peers": reported_peers,
        "versions": current["versions"],
        "cautions": caution,
        "limitations": [
            "1.1.1 and 1.1.2 share the "
            "post-500000 consensus rules.",
            "Version labels can be stale, "
            "duplicated or spoofed.",
            "One website source is not an "
            "independent network census.",
            "No remote node is upgraded, "
            "blocked or modified.",
            "No keys, IPs, addresses or "
            "wallet data are collected.",
        ],
    }


def write_evidence(result, out):

    out.mkdir(
        parents=True,
        exist_ok=True,
        mode=0o700,
    )

    payload = json.dumps(
        result,
        indent=2,
        sort_keys=True,
    ) + "\n"

    nw.atomic_write(
        out / "upgrade-gate.json",
        payload.encode("utf-8"),
    )

    lines = [
        "# Nightwatch Post-Activation Upgrade Gate",
        "",
        "## Observation",
        "",
        "- Time: " + result["latest_sample_utc"],
        "- Height: " + str(result["latest_height"]),
        "- Status: **" + result["status"] + "**",
        "- Legacy labels: " + str(
            result["reported_legacy_labels"]
        ),
        "- Compatible 1.1.x labels: " + str(
            result["reported_compatible_labels"]
        ),
        "- Unknown version labels: " + str(
            result["reported_unclassified_labels"]
        ),
        "- Samples considered: " + str(
            result["observations_considered"]
        ),
        "- Persistent legacy labels: " + str(
            result["persistent_legacy_labels"]
        ),
        "",
        "## Cautions",
        "",
    ]

    if result["cautions"]:
        lines.extend(
            "- " + x
            for x in result["cautions"]
        )
    else:
        lines.append("- None in last sample")

    lines.extend([
        "",
        "## Interpretation",
        "",
    ])

    lines.extend(
        "- " + x
        for x in result["limitations"]
    )

    nw.atomic_write(
        out / "upgrade-gate.md",
        ("\n".join(lines) + "\n").encode("utf-8"),
    )


def main():

    parser = argparse.ArgumentParser(
        description=__doc__
    )

    parser.add_argument(
        "--history",
        type=Path,
        default=nw.OUT / "history.json",
    )

    parser.add_argument(
        "--output",
        type=Path,
        default=nw.OUT,
    )

    args = parser.parse_args()

    if not args.history.is_file():
        print(
            "NO DATA: run nightwatch.py first",
            file=sys.stderr,
        )
        return 2

    if args.history.stat().st_size > 8000000:
        print(
            "INVALID DATA: history too large",
            file=sys.stderr,
        )
        return 2

    try:

        rows = json.loads(
            args.history.read_text(
                encoding="utf-8"
            )
        )

        report = evaluate(rows)

        write_evidence(
            report,
            args.output,
        )

    except (
        ValueError,
        KeyError,
        TypeError,
        OSError,
    ) as exc:

        print(
            "INVALID DATA:",
            exc,
            file=sys.stderr,
        )
        return 2

    print(
        "UPGRADE GATE:",
        report["status"],
    )

    print(
        "Height:",
        report["latest_height"],
    )

    print(
        "Legacy labels:",
        report["reported_legacy_labels"],
    )

    print(
        "Compatible labels:",
        report["reported_compatible_labels"],
    )

    print(
        "Unknown labels:",
        report["reported_unclassified_labels"],
    )

    print(
        "Persistent over 10 minutes:",
        report["persistent_legacy_labels"],
    )

    print(
        "Evidence:",
        args.output / "upgrade-gate.md",
    )

    return 0


if __name__ == "__main__":
    sys.exit(main())
