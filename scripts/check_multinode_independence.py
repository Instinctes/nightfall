#!/usr/bin/env python3
"""Fail-closed repeat runner for real-process Devnet tests."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]

TEST_FILE = (
    ROOT
    / "crates/nightfall-node/tests/multinode_fault_harness.rs"
)

EXPECTED = {
    "partition_cuts_real_session_and_heal_resynchronizes",
    "partitioned_competing_chains_converge_on_heavier_work",
    "three_node_survival_after_origin_failure",
}

MAX_CAPTURE = 2 * 1024 * 1024


def finished_tests(output):
    return set(
        re.findall(
            r"(?m)^test\s+(\w+)\s+\.\.\.\s+ok\s*$",
            output,
        )
    )


def self_test():
    sample = "\n".join(
        f"test {name} ... ok"
        for name in sorted(EXPECTED)
    )

    assert finished_tests(sample) == EXPECTED

    failed_sample = sample.replace(
        "... ok", "... FAILED", 1
    )

    assert finished_tests(failed_sample) != EXPECTED

    assert finished_tests(
        "test result: ok. 0 passed; 0 failed"
    ) == set()

    print("SELF-TEST OK")


def version(command):
    return subprocess.check_output(
        command,
        cwd=ROOT,
        text=True,
    ).strip()


def fingerprint():
    return {
        "source_commit": version(
            ["git", "rev-parse", "HEAD"]
        ),
        "test_file_sha256": hashlib.sha256(
            TEST_FILE.read_bytes()
        ).hexdigest(),
        "cargo_lock_sha256": hashlib.sha256(
            (ROOT / "Cargo.lock").read_bytes()
        ).hexdigest(),
        "rustc": version(["rustc", "--version"]),
        "cargo": version(["cargo", "--version"]),
    }


def write_receipt(path, receipt):
    path.parent.mkdir(
        parents=True,
        exist_ok=True,
    )

    path.write_text(
        json.dumps(
            receipt,
            indent=2,
            sort_keys=True,
        ) + "\n",
        encoding="utf-8",
    )


def main():
    ap = argparse.ArgumentParser(
        description=__doc__
    )

    ap.add_argument(
        "--self-test",
        action="store_true",
    )

    ap.add_argument(
        "--repeats",
        type=int,
        default=3,
    )

    ap.add_argument(
        "--timeout-seconds",
        type=int,
        default=900,
    )

    ap.add_argument(
        "--report",
        type=Path,
    )

    args = ap.parse_args()

    if args.self_test:
        self_test()
        return 0

    if not 1 <= args.repeats <= 10:
        ap.error("repeats must be 1..10")

    if not 60 <= args.timeout_seconds <= 3600:
        ap.error("timeout must be 60..3600")

    receipt = {
        "scope": "isolated-devnet-real-process-tests",
        "disclaimer": (
            "Not an independent security audit "
            "or proof of real operator diversity"
        ),
        "date_utc": datetime.now(
            timezone.utc
        ).isoformat(),
        "source": fingerprint(),
        "expected_tests": sorted(EXPECTED),
        "runs": [],
        "passed": False,
    }

    cmd = [
        "cargo",
        "test",
        "--locked",
        "-p",
        "nightfall-node",
        "--test",
        "multinode_fault_harness",
        "--",
        "--test-threads=1",
    ]

    for index in range(1, args.repeats + 1):
        started = time.monotonic()

        code = None
        timed_out = False

        with tempfile.TemporaryFile(
            mode="w+t",
            encoding="utf-8",
        ) as logfile:

            process = subprocess.Popen(
                cmd,
                cwd=ROOT,
                stdout=logfile,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )

            try:
                code = process.wait(
                    timeout=args.timeout_seconds
                )

            except subprocess.TimeoutExpired:
                timed_out = True

                try:
                    os.killpg(
                        process.pid,
                        signal.SIGKILL,
                    )
                except ProcessLookupError:
                    pass

                process.wait()

            logfile.seek(0)

            output = logfile.read(
                MAX_CAPTURE + 1
            )

        truncated = len(output) > MAX_CAPTURE

        successful_tests = finished_tests(output)

        passed = (
            code == 0
            and not timed_out
            and not truncated
            and EXPECTED.issubset(successful_tests)
            and bool(
                re.search(
                    r"(?m)^test result: ok\.",
                    output,
                )
            )
        )

        record = {
            "run": index,
            "duration_seconds": round(
                time.monotonic() - started,
                2,
            ),
            "exit_code": code,
            "timed_out": timed_out,
            "output_truncated": truncated,
            "passed_tests": sorted(
                successful_tests & EXPECTED
            ),
            "passed": passed,
        }

        receipt["runs"].append(record)

        print(
            f"DEVNET RUN {index}/{args.repeats}: "
            f"{'PASS' if passed else 'FAIL'} "
            f"{record['duration_seconds']}s",
            flush=True,
        )

        if args.report:
            write_receipt(
                args.report,
                receipt,
            )

        if not passed:
            print(
                "STOP: missing, failed or timed-out test.",
                file=sys.stderr,
            )
            return 1

    receipt["passed"] = True

    if args.report:
        write_receipt(
            args.report,
            receipt,
        )

    print(
        "PASS: all three scenarios passed "
        "in every repetition"
    )

    return 0


if __name__ == "__main__":
    sys.exit(main())
