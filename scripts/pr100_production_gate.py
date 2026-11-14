#!/usr/bin/env python3
"""Fail-closed offline production evidence for Nightfall PR100."""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

from datetime import datetime, timezone
from pathlib import Path

GIB = 1024 ** 3
MAX_FILES = 1_000_000
MAX_BYTES = 128 * GIB
MARKER = "PR100_REAL_MAINNET_PASS"


def fingerprint(folder):
    """Read-only SHA256 identity of an offline fixture."""

    folder = Path(folder)

    if folder.is_symlink() or not folder.is_dir():
        raise ValueError(
            "Fixture missing, invalid or symlinked"
        )

    digest = hashlib.sha256()
    count = 0
    total = 0

    for root, dirs, files in os.walk(
        folder,
        topdown=True,
        followlinks=False
    ):
        dirs.sort()
        files.sort()

        for name in dirs:
            p = Path(root) / name
            if p.is_symlink():
                raise ValueError(
                    "Directory symlink rejected"
                )

        for name in files:
            p = Path(root) / name

            if p.is_symlink() or not p.is_file():
                raise ValueError(
                    "Link or special file rejected"
                )

            st = p.stat(follow_symlinks=False)

            if st.st_nlink != 1:
                raise ValueError(
                    "Hardlinked file rejected"
                )

            count += 1
            total += st.st_size

            if count > MAX_FILES or total > MAX_BYTES:
                raise ValueError(
                    "Fixture exceeds safety bounds"
                )

            relative = p.relative_to(folder)
            encoded = relative.as_posix().encode("utf-8")

            digest.update(
                len(encoded).to_bytes(4, "big")
            )
            digest.update(encoded)
            digest.update(
                st.st_size.to_bytes(8, "big")
            )

            with p.open("rb") as f:
                for chunk in iter(
                    lambda: f.read(1024 * 1024),
                    b""
                ):
                    digest.update(chunk)

            if p.stat(
                follow_symlinks=False
            ).st_size != st.st_size:
                raise ValueError(
                    "Fixture changed while hashing"
                )

    if count == 0:
        raise ValueError("Empty fixture")

    return {
        "sha256_tree": digest.hexdigest(),
        "files": count,
        "bytes": total
    }


def parse_mainnet_pass(output):
    """Fail closed on malformed or contradictory real-data test receipts.

    A consistent log is a local test receipt, never independent chain proof.
    """
    # pr100-evidence-consistency-v2
    import math

    required = (
        "checkpoint", "source_tip", "catchup", "snapshot_bytes",
        "load_s", "export_s", "import_s", "catchup_s"
    )
    allowed = set(required) | {"peak_ram_kib"}
    lines = [line.strip() for line in output.splitlines() if MARKER in line]
    if len(lines) != 1 or len(lines[0]) > 2048:
        return None
    tokens = lines[0].split()
    if not tokens or tokens[0] != MARKER:
        return None

    values = {}
    for token in tokens[1:]:
        if token.count("=") != 1:
            return None
        key, raw = token.split("=", 1)
        if key not in allowed or key in values or not raw:
            return None
        values[key] = raw
    if not all(k in values for k in required):
        return None

    def unsigned(key):
        raw = values[key]
        return int(raw) if re.fullmatch(r"[0-9]+", raw) else None

    checkpoint = unsigned("checkpoint")
    tip = unsigned("source_tip")
    catchup = unsigned("catchup")
    snapshot = unsigned("snapshot_bytes")
    if any(x is None for x in (checkpoint, tip, catchup, snapshot)):
        return None
    if (checkpoint < 505000 or tip <= checkpoint
            or catchup != tip - checkpoint or catchup <= 0
            or not 0 < snapshot <= 512 * 1024 * 1024):
        return None

    for key in ("load_s", "export_s", "import_s", "catchup_s"):
        raw = values[key]
        if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", raw):
            return None
        measured = float(raw)
        if not math.isfinite(measured) or measured < 0:
            return None

    ram = values.get("peak_ram_kib", "None")
    if ram != "None" and not re.fullmatch(r"Some\([0-9]+\)", ram):
        return None
    return {key: values[key] for key in (*required, "peak_ram_kib") if key in values}

def run_logged(command, repo, log_path,
               env=None, timeout=3600):

    try:
        with log_path.open(
            "w", encoding="utf-8"
        ) as log:
            result = subprocess.run(
                command,
                cwd=repo,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
                timeout=timeout,
                check=False
            )

        output = log_path.read_text(
            encoding="utf-8",
            errors="replace"
        )

        return result.returncode, output

    except (OSError, subprocess.TimeoutExpired) as e:
        with log_path.open(
            "a", encoding="utf-8"
        ) as log:
            log.write(
                "\nRUNNER ERROR: " +
                type(e).__name__ + "\n"
            )

        return 125, ""


def resources_allow(source_bytes, destination):
    """Avoid exhausting a resource-constrained phone."""

    mem = 0

    try:
        lines = Path(
            "/proc/meminfo"
        ).read_text().splitlines()

        for line in lines:
            if line.startswith("MemAvailable:"):
                mem = int(line.split()[1]) * 1024
                break

    except (OSError, ValueError):
        pass

    free = shutil.disk_usage(destination).free

    needed = max(
        3 * GIB,
        2 * source_bytes + GIB
    )

    return (
        mem >= 2 * GIB and free >= needed,
        {
            "ram_available_bytes": mem,
            "free_disk_bytes": free,
            "minimum_disk_bytes": needed
        }
    )


def check_legacy(repo, fixture, evidence):
    """Prove unforced refusal preserves fixture bytes."""

    if not fixture.exists():
        return {
            "state": "BLOCKED",
            "reason": "No genuine offline 1.1.2 operator fixture"
        }

    try:
        before = fingerprint(fixture)
    except (OSError, ValueError) as e:
        return {
            "state": "FAILED",
            "reason": str(e)
        }

    if not shutil.which("cargo"):
        return {
            "state": "BLOCKED",
            "reason": "Cargo unavailable",
            "fixture": before
        }

    allowed, resources = resources_allow(
        before["bytes"],
        evidence
    )

    if not allowed:
        return {
            "state": "BLOCKED",
            "reason": "Insufficient device resources",
            "fixture": before,
            "resources": resources
        }

    command = [
        "cargo", "run",
        "--locked",
        "--quiet",
        "-p", "nightfall-nodekit",
        "--",
        "--network", "mainnet",
        "upgrade-pruned",
        "--datadir", str(fixture)
    ]

    rc, output = run_logged(
        command,
        repo,
        evidence / "legacy-refusal.log",
        timeout=1800
    )

    try:
        after = fingerprint(fixture)
    except (OSError, ValueError) as e:
        return {
            "state": "FAILED",
            "reason": "Fixture unreadable: " + str(e)
        }

    if before != after:
        return {
            "state": "FAILED",
            "reason": "Fixture bytes changed"
        }

    if rc == 0:
        return {
            "state": "FAILED",
            "reason": "Unsafe unforced upgrade succeeded"
        }

    valid_refusal = (
        "No files were changed" in output
        or
        "Legacy pruned 1.1.x state" in output
    )

    if not valid_refusal:
        return {
            "state": "FAILED",
            "reason": "Expected refusal not demonstrated",
            "process_exit": rc
        }

    return {
        "state": "PASS_LIMITED",
        "reason": "Unforced refusal and byte preservation only",
        "fixture": before
    }


def check_mainnet(repo, fixture, evidence):
    """Execute existing real-data Rust acceptance harness."""

    if not fixture.exists():
        return {
            "state": "BLOCKED",
            "reason": "No complete offline Mainnet archive fixture"
        }

    try:
        before = fingerprint(fixture)
    except (OSError, ValueError) as e:
        return {
            "state": "FAILED",
            "reason": str(e)
        }

    if not shutil.which("cargo"):
        return {
            "state": "BLOCKED",
            "reason": "Cargo unavailable",
            "fixture": before
        }

    allowed, resources = resources_allow(
        before["bytes"],
        evidence
    )

    if not allowed:
        return {
            "state": "BLOCKED",
            "reason": "Insufficient device resources",
            "fixture": before,
            "resources": resources
        }

    env = os.environ.copy()

    env["NF_PR100_ARCHIVE_COPY"] = str(fixture)
    env["CARGO_BUILD_JOBS"] = "2"
    env["CARGO_INCREMENTAL"] = "0"

    command = [
        "cargo", "test",
        "--locked",
        "-p", "nightfall-storage",
        "--test",
        "pr100_real_checkpoint_acceptance",
        "--",
        "--ignored",
        "--nocapture"
    ]

    rc, output = run_logged(
        command,
        repo,
        evidence / "mainnet-acceptance.log",
        env=env,
        timeout=3600
    )

    try:
        after = fingerprint(fixture)
    except (OSError, ValueError) as e:
        return {
            "state": "FAILED",
            "reason": "Fixture unreadable: " + str(e)
        }

    if before != after:
        return {
            "state": "FAILED",
            "reason": "Mainnet source bytes changed"
        }

    measurements = (
        parse_mainnet_pass(output)
        if rc == 0
        else None
    )

    if not measurements or "1 passed;" not in output:
        return {
            "state": "FAILED",
            "reason": "Real Rust acceptance did not pass",
            "process_exit": rc,
            "fixture": before
        }

    return {
        "state": "PASS_LIMITED",
        "reason": "Local real-data equivalence only",
        "fixture": before,
        "observed": measurements
    }


def main():

    repo = Path(__file__).resolve().parents[1]

    fixtures = (
        Path.home() /
        "nightfall-pr100-fixtures"
    )

    evidence = (
        Path.home() /
        "nightfall-pr100-evidence"
    )

    evidence.mkdir(
        parents=True,
        exist_ok=True,
        mode=0o700
    )

    os.chmod(evidence, 0o700)

    rust_test = (
        repo /
        "crates/nightfall-storage/tests/"
        "pr100_real_checkpoint_acceptance.rs"
    )

    if not rust_test.is_file():
        print(
            "STOP: PR100 real acceptance test missing",
            file=sys.stderr
        )
        return 2

    report = {
        "generated_utc": datetime.now(
            timezone.utc
        ).isoformat(),

        "purpose": (
            "PR100 production evidence with explicit limits"
        ),

        "source_fixture_policy": (
            "Disposable offline copies only"
        ),

        "legacy_112_refusal": check_legacy(
            repo,
            fixtures / "untouched-1.1.2-pruned",
            evidence
        ),

        "mainnet_export_import_catchup": check_mainnet(
            repo,
            fixtures / "complete-mainnet-archive",
            evidence
        ),

        "independent_checkpoint_approval": {
            "state": "BLOCKED",
            "reason": "Independent operator review required"
        },

        "desktop_core_gui_recovery": {
            "state": "BLOCKED",
            "reason": "Real desktop GUI recovery required"
        },

        "mainnet_interrupted_import_rehearsal": {
            "state": "BLOCKED",
            "reason": "Real-data interruption test not supplied"
        },

        "representative_cross_platform_benchmarks": {
            "state": "BLOCKED",
            "reason": "Desktop/server measurements required"
        },

        "maintainer_security_release_approval": {
            "state": "BLOCKED",
            "reason": "Independent review and maintainer approval required"
        }
    }

    output = evidence / "pr100-latest.json"
    temporary = evidence / "pr100-latest.json.tmp"

    temporary.write_text(
        json.dumps(
            report,
            indent=2,
            ensure_ascii=False
        ) + "\n",
        encoding="utf-8"
    )

    os.chmod(temporary, 0o600)
    temporary.replace(output)

    print("\n=== PR100 PRODUCTION EVIDENCE ===")

    for key, value in report.items():
        if isinstance(value, dict) and "state" in value:
            print(
                f"{key}: {value['state']} - "
                f"{value['reason']}"
            )

    print("\nLocal private report:", output)
    print("No Mainnet release approval claimed.")

    failures = [
        value
        for value in report.values()
        if isinstance(value, dict)
        and value.get("state") == "FAILED"
    ]

    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
