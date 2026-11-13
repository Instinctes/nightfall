#!/usr/bin/env python3
"""Nightwatch: privacy-minimal public Nightfall network history and evidence."""

import argparse
from datetime import datetime, timezone
import hashlib
import html
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time
import urllib.error
import urllib.request

URL = "https://nightfallcoin.org/network.json"

GENESIS = (
    "061a052d49607ff8f4b306c75d622eb"
    "d230cff4ec3a45a6dffc2f7738d4b20de"
)

MAX_SUPPLY = 90_000_000 * 100_000_000
ERA = 7_500_000
REWARD = 600_000_000
MAX_HISTORY = 4000

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"

HEX64 = re.compile(r"[0-9a-f]{64}\Z")

VERSION = re.compile(
    r"nightfalld/[0-9]+\.[0-9]+\.[0-9]+\Z"
)


def integer(obj, key, ceiling=10**30):
    value = obj.get(key)

    if type(value) is not int or not 0 <= value <= ceiling:
        raise ValueError(
            f"invalid integer field: {key}"
        )

    return value


def exact_hex(value, key):
    if (
        not isinstance(value, str)
        or not HEX64.fullmatch(value)
    ):
        raise ValueError(
            f"invalid hash: {key}"
        )

    return value


def expected_minted(blocks):
    remaining = blocks
    total = 0

    for era in range(30):
        if remaining <= 0:
            break

        count = min(remaining, ERA)

        total += count * (REWARD >> era)
        remaining -= count

    return total


def sanitize(raw):
    """
    Explicit allowlist.
    Never persist unknown fields, addresses or IPs.
    """

    if not isinstance(raw, dict):
        raise ValueError(
            "JSON root is not an object"
        )

    for key, expected in (
        ("network", "mainnet"),
        ("ticker", "NIGHT"),
        ("genesis", GENESIS),
    ):
        if raw.get(key) != expected:
            raise ValueError(
                f"unexpected {key}; foreign network refused"
            )

    p = {
        "network": "mainnet",
        "ticker": "NIGHT",
        "genesis": GENESIS,
    }

    for key in (
        "protocol_version",
        "wire_version",
        "blocks",
        "tip_height",
        "difficulty",
        "circulating",
        "minted",
        "burned_fees",
        "max_supply",
        "peers",
        "utxos",
        "kernels",
        "mempool",
        "tip_time",
    ):
        p[key] = integer(raw, key)

    p["tip"] = exact_hex(
        raw.get("tip"), "tip"
    )

    p["utxo_root"] = exact_hex(
        raw.get("utxo_root"), "utxo_root"
    )

    tw = raw.get("total_work")

    if (
        not isinstance(tw, str)
        or not re.fullmatch(
            r"(?:0|[1-9][0-9]{0,29})", tw
        )
    ):
        raise ValueError(
            "invalid total_work decimal string"
        )

    p["total_work"] = tw

    if type(raw.get("supply_invariant_ok")) is not bool:
        raise ValueError(
            "invalid supply_invariant_ok"
        )

    p["supply_invariant_ok"] = raw[
        "supply_invariant_ok"
    ]

    versions = raw.get("peer_versions")

    if (
        not isinstance(versions, dict)
        or len(versions) > 40
    ):
        raise ValueError(
            "invalid peer_versions"
        )

    cleaned = {}

    for key, count in versions.items():

        if (
            not isinstance(key, str)
            or not VERSION.fullmatch(key)
        ):
            continue

        if (
            type(count) is int
            and 0 <= count <= 100000
        ):
            cleaned[key] = count

    p["peer_versions"] = dict(
        sorted(cleaned.items())
    )

    return p


def review(p, prior=None, now=None):

    now = int(
        time.time() if now is None else now
    )

    critical = []
    caution = []

    if (
        p["protocol_version"] != 8
        or p["wire_version"] != 6
    ):
        critical.append(
            "Unexpected protocol or wire version"
        )

    if p["blocks"] != p["tip_height"] + 1:
        critical.append(
            "Block count and tip height disagree"
        )

    if p["max_supply"] != 90_000_000:
        critical.append(
            "Reported supply ceiling differs"
        )

    if p["minted"] != expected_minted(
        p["blocks"]
    ):
        critical.append(
            "Reported issuance differs from integer schedule"
        )

    if (
        p["minted"] > MAX_SUPPLY
        or p["burned_fees"] > p["minted"]
    ):
        critical.append(
            "Impossible supply totals"
        )

    if p["circulating"] != (
        p["minted"] - p["burned_fees"]
    ):
        critical.append(
            "Minted minus burned differs from circulating"
        )

    if not p["supply_invariant_ok"]:
        critical.append(
            "Node reports supply invariant failure"
        )

    if p["tip_time"] > now + 180:
        caution.append(
            "Block timestamp exceeds local clock by 180 seconds"
        )

    if now - p["tip_time"] > 600:
        caution.append(
            "Reported block older than 10 minutes"
        )

    legacy = {}

    for key, count in p["peer_versions"].items():

        version = tuple(
            map(
                int,
                key.split("/")[1].split(".")
            )
        )

        if version < (1, 1, 1) and count:
            legacy[key] = count

    if legacy:
        caution.append(
            "Legacy node version labels observed"
        )

    if prior:

        if p["tip_height"] < prior["tip_height"]:
            caution.append(
                "Height decreased; possible reorg or stale source"
            )

        if (
            p["tip_height"] == prior["tip_height"]
            and p["tip"] != prior["tip"]
        ):
            caution.append(
                "Conflicting tip hashes at equal height"
            )

        if (
            p["tip_height"] > prior["tip_height"]
            and int(p["total_work"])
                < int(prior["total_work"])
        ):
            critical.append(
                "Reported cumulative work decreased"
            )

    if critical:
        level = "CRITICAL"
    elif caution:
        level = "CAUTION"
    else:
        level = "OBSERVED"

    return {
        "level": level,
        "critical": critical,
        "caution": caution,
        "limits": [
            "Single website-node claim, not independent chain validation",
            "Peer version labels do not measure independent operators",
            "Pool concentration UNKNOWN without public attributable data",
            "SHA-256 journal is not a digital signature",
        ],
    }


def canonical(obj):

    return json.dumps(
        obj,
        sort_keys=True,
        ensure_ascii=True,
        separators=(",", ":"),
    ).encode("ascii")


def validate_history(rows):

    if (
        not isinstance(rows, list)
        or len(rows) > MAX_HISTORY
    ):
        raise ValueError(
            "history shape/length invalid"
        )

    prev = "0" * 64

    for row in rows:

        if (
            not isinstance(row, dict)
            or set(row) != {
                "time_utc",
                "claim",
                "previous_sha256",
                "sha256",
            }
        ):
            raise ValueError(
                "history entry shape invalid"
            )

        if row["previous_sha256"] != prev:
            raise ValueError(
                "history hash link broken"
            )

        payload = {
            k: row[k]
            for k in (
                "time_utc",
                "claim",
                "previous_sha256",
            )
        }

        digest = hashlib.sha256(
            canonical(payload)
        ).hexdigest()

        if digest != row["sha256"]:
            raise ValueError(
                "history checksum changed"
            )

        prev = digest

    return True


def seal(rows):

    prev = "0" * 64
    sealed = []

    for row in rows[-MAX_HISTORY:]:

        entry = {
            "time_utc": row["time_utc"],
            "claim": row["claim"],
            "previous_sha256": prev,
        }

        entry["sha256"] = hashlib.sha256(
            canonical(entry)
        ).hexdigest()

        sealed.append(entry)
        prev = entry["sha256"]

    return sealed


def atomic_write(path, data):

    path.parent.mkdir(
        parents=True,
        exist_ok=True,
        mode=0o700,
    )

    fd, tmp = tempfile.mkstemp(
        prefix=".nw-",
        dir=path.parent,
    )

    try:

        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())

        os.chmod(tmp, 0o600)
        os.replace(tmp, path)

    finally:

        if os.path.exists(tmp):
            os.unlink(tmp)


def load_history():

    path = OUT / "history.json"

    if not path.exists():
        return []

    if path.stat().st_size > 8_000_000:
        raise ValueError(
            "history file unexpectedly large"
        )

    rows = json.loads(
        path.read_text(encoding="utf-8")
    )

    validate_history(rows)

    return rows


def fetch():

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
            newurl,
        ):
            raise ValueError(
                "HTTP redirect refused"
            )

    opener = urllib.request.build_opener(
        RefuseRedirect()
    )

    req = urllib.request.Request(
        URL,
        headers={
            "Accept": "application/json",
            "User-Agent": "Nightwatch-1.0 read-only",
        },
    )

    with opener.open(req, timeout=12) as response:

        if response.status != 200:
            raise ValueError(
                f"HTTP {response.status}"
            )

        data = response.read(32769)

    if len(data) > 32768:
        raise ValueError(
            "Public response too large"
        )

    return sanitize(
        json.loads(data.decode("utf-8"))
    )


def make_reports(rows):

    if not rows:
        return

    p = rows[-1]["claim"]

    prior = (
        rows[-2]["claim"]
        if len(rows) > 1
        else None
    )

    result = review(p, prior)

    summary = {
        "scope": "read-only public network observations",
        "observation_count": len(rows),
        "latest_observation_utc": rows[-1]["time_utc"],
        "source": URL,
        "last_hash": rows[-1]["sha256"],
        "assessment": result,
        "latest": p,
        "externally_verified": False,
    }

    atomic_write(
        OUT / "report.json",
        json.dumps(
            summary,
            indent=2,
            sort_keys=True,
        ).encode() + b"\n",
    )

    md = [
        "# Nightfall Nightwatch — public network observations",
        "",
        f"- Sampled at: {rows[-1]['time_utc']}",
        f"- Assessment: **{result['level']}**",
        f"- Height: {p['tip_height']}",
        f"- Tip hash: `{p['tip']}`",
        f"- Total work (node claim): {p['total_work']}",
        f"- Reported supply invariant: {p['supply_invariant_ok']}",
        f"- Minted (darks): {p['minted']}",
        f"- Burned fees (darks): {p['burned_fees']}",
        f"- Circulating (darks): {p['circulating']}",
        "- Independent node operator count: **UNKNOWN**",
        "- Mining pool concentration: **UNKNOWN**",
        "",
        "## Flags",
        "",
    ]

    md.extend(
        f"- CRITICAL: {x}"
        for x in result["critical"]
    )

    md.extend(
        f"- Caution: {x}"
        for x in result["caution"]
    )

    if (
        not result["critical"]
        and not result["caution"]
    ):
        md.append(
            "- No inconsistency detected in reported fields."
        )

    md.extend([
        "",
        "## Interpretation",
        "",
    ])

    md.extend(
        f"- {x}"
        for x in result["limits"]
    )

    md.extend([
        "",
        "## Known node-version labels",
        "",
    ])

    md.extend(
        f"- {k}: {v}"
        for k, v in p["peer_versions"].items()
    )

    md.extend([
        "",
        "## Observation receipt",
        "",
        f"`{rows[-1]['sha256']}`",
        "",
    ])

    atomic_write(
        OUT / "report.md",
        "\n".join(md).encode("utf-8"),
    )

    points = rows[-60:]

    heights = [
        x["claim"]["tip_height"]
        for x in points
    ]

    low = min(heights)
    high = max(heights)

    width = 700
    height = 130

    poly = " ".join(
        f"{(i * width // max(1, len(points)-1))},"
        f"{int(height - 8 - (h-low)*110//max(1, high-low))}"
        for i, h in enumerate(heights)
    )

    flags = "".join(
        f"<li>{html.escape(x)}</li>"
        for x in (
            result["critical"]
            + result["caution"]
        )
    ) or "<li>No inconsistency detected</li>"

    table = "".join(
        f"<tr><td>{html.escape(k)}</td><td>{v}</td></tr>"
        for k, v in p["peer_versions"].items()
    )

    page = f"""<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src 'none'; form-action 'none'">
<title>Nightwatch | Nightfall network evidence</title>
<style>
body {{
 background:#0a1017;
 color:#e8edf3;
 font:16px system-ui;
 margin:auto;
 max-width:850px;
 padding:25px
}}
h1 {{color:#b9e3ff}}
.tiles {{display:flex;flex-wrap:wrap;gap:12px}}
.tile {{
 border:1px solid #405268;
 border-radius:12px;
 padding:18px;
 flex:1;
 min-width:170px
}}
small {{color:#abc0d5}}
strong {{display:block;font-size:1.5em}}
svg {{
 width:100%;
 height:auto;
 background:#122130;
 border-radius:8px
}}
table {{width:100%;border-collapse:collapse}}
td {{padding:8px;border-bottom:1px solid #34475a}}
.note {{color:#fbcf85}}
</style>
<h1>NIGHTWATCH / PUBLIC NETWORK</h1>
<p>Read-only · {html.escape(rows[-1]['time_utc'])} · {len(rows)} observations</p>

<div class="tiles">
 <div class="tile">
  <small>Assessment</small>
  <strong>{result['level']}</strong>
 </div>
 <div class="tile">
  <small>Block height</small>
  <strong>{p['tip_height']}</strong>
 </div>
 <div class="tile">
  <small>Peer connections (node claim)</small>
  <strong>{p['peers']}</strong>
 </div>
 <div class="tile">
  <small>Supply claim</small>
  <strong>{'OK' if p['supply_invariant_ok'] else 'FAILED'}</strong>
 </div>
</div>

<h2>Observed height history</h2>
<svg viewBox="0 0 700 130" role="img" aria-label="Height trend from sampled observations">
 <polyline points="{poly}" fill="none" stroke="#70d2a8" stroke-width="3"/>
</svg>

<h2>Flags</h2>
<ul>{flags}</ul>

<h2>Reported software labels</h2>
<table>{table}</table>

<p class="note">
Mining pool concentration: UNKNOWN.
Node counts and version labels do not establish independent operators.
Source reports are not independent cryptographic verification.
No addresses, keys, wallet activity or IP tracking.
</p>
</html>
"""

    atomic_write(
        OUT / "report.html",
        page.encode("utf-8"),
    )

    return summary


def sample():

    p = fetch()
    rows = load_history()

    now = datetime.now(
        timezone.utc
    ).isoformat(timespec="seconds")

    new = seal([
        *rows,
        {
            "time_utc": now,
            "claim": p,
        },
    ])

    validate_history(new)

    atomic_write(
        OUT / "history.json",
        json.dumps(
            new,
            indent=1,
        ).encode("utf-8"),
    )

    report = make_reports(new)

    # Nightwatch automatic upgrade evidence.
    try:
        import nightwatch_upgrade_gate as gate
        gate.write_evidence(
            gate.evaluate(new),
            OUT,
        )
    except (
        ImportError,
        ValueError,
        KeyError,
        TypeError,
        OSError,
    ) as exc:
        print(
            "UPGRADE GATE FAILED: "
            f"{type(exc).__name__}: {exc}",
            file=sys.stderr,
            flush=True,
        )

    print(
        f"[{now}] "
        f"{report['assessment']['level']} "
        f"height={p['tip_height']} "
        f"supply_claim={p['supply_invariant_ok']} "
        f"samples={len(new)}",
        flush=True,
    )

    return report


def main():

    ap = argparse.ArgumentParser(
        description=__doc__
    )

    mode = ap.add_mutually_exclusive_group()

    mode.add_argument(
        "--watch",
        action="store_true",
        help="Run repeating safe read-only samples",
    )

    mode.add_argument(
        "--verify",
        action="store_true",
        help="Verify existing local evidence offline",
    )

    ap.add_argument(
        "--interval",
        type=int,
        default=300,
    )

    args = ap.parse_args()

    if args.verify:

        rows = load_history()

        print(
            f"JOURNAL OK: {len(rows)} observations "
            "(not digitally signed)"
        )

        return 0

    if (
        args.watch
        and not 60 <= args.interval <= 86400
    ):
        ap.error(
            "interval must be 60..86400 seconds"
        )

    while True:

        try:

            result = sample()

            if not args.watch:
                return (
                    1
                    if result["assessment"]["level"] == "CRITICAL"
                    else 0
                )

        except (
            OSError,
            ValueError,
            json.JSONDecodeError,
            urllib.error.URLError,
        ) as exc:

            print(
                "DATA UNAVAILABLE / "
                "NO VERIFIED LIVE OBSERVATION: "
                f"{exc}",
                file=sys.stderr,
                flush=True,
            )

            if not args.watch:
                return 2

        if not args.watch:
            break

        time.sleep(args.interval)

    return 0


if __name__ == "__main__":
    sys.exit(main())
