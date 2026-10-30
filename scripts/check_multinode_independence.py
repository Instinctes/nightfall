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
import selectors
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

    # Success, flooding, timeout, failure.

    good = run_bounded(
        [
            sys.executable,
            "-c",
            "print('ok')",
        ],
        15,
        128,
    )

    assert good == (
        0,
        False,
        False,
        "ok\n",
    )

    flood = run_bounded(
        [
            sys.executable,
            "-c",
            "print('X' * 20000)",
        ],
        15,
        128,
    )

    assert flood[2]
    assert not flood[1]

    assert (
        len(flood[3].encode("utf-8"))
        <= 128
    )

    hang = run_bounded(
        [
            sys.executable,
            "-c",
            "import time; time.sleep(8)",
        ],
        1,
        128,
    )

    assert hang[1]
    assert not hang[2]

    failing = run_bounded(
        [
            sys.executable,
            "-c",
            "import sys; sys.exit(7)",
        ],
        15,
        128,
    )

    assert failing[0] == 7
    assert not failing[1]
    assert not failing[2]

    print("SELF-TEST OK")


def run_bounded(
    command,
    timeout_seconds,
    max_capture=MAX_CAPTURE,
):
    """Bound captured output and child lifetime."""

    deadline = time.monotonic() + timeout_seconds

    child = subprocess.Popen(
        command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )

    collected = bytearray()

    timed_out = False
    truncated = False

    try:
        with selectors.DefaultSelector() as events:

            events.register(
                child.stdout,
                selectors.EVENT_READ,
            )

            while events.get_map():

                remaining = (
                    deadline - time.monotonic()
                )

                if remaining <= 0:
                    timed_out = True
                    break

                ready = events.select(
                    min(0.25, remaining)
                )

                for key, _ in ready:

                    budget = (
                        max_capture - len(collected)
                    )

                    chunk = os.read(
                        key.fileobj.fileno(),
                        min(65536, budget + 1),
                    )

                    if not chunk:
                        events.unregister(key.fileobj)
                        continue

                    collected.extend(
                        chunk[:budget]
                    )

                    if len(chunk) > budget:
                        truncated = True
                        break

                if truncated:
                    break

        if not timed_out and not truncated:

            try:
                child.wait(
                    timeout=max(
                        0,
                        deadline - time.monotonic(),
                    )
                )

            except subprocess.TimeoutExpired:
                timed_out = True

    finally:

        if (
            timed_out
            or truncated
            or child.poll() is None
        ):

            try:
                os.killpg(
                    child.pid,
                    signal.SIGKILL,
                )

            except ProcessLookupError:
                pass

        child.wait()
        child.stdout.close()

    return (
        child.returncode,
        timed_out,
        truncated,
        collected.decode(
            "utf-8",
            errors="replace",
        ),
    )


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
        "--ignored",
        "--test-threads=1",
    ]

    for index in range(1, args.repeats + 1):
        started = time.monotonic()

        code = None
        timed_out = False

        code, timed_out, truncated, output = run_bounded(
            cmd, args.timeout_seconds
        )

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
