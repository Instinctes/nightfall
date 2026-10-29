#!/usr/bin/env python3

"""Repeat the real short-page Devnet integration test, fail closed.

No production RPC, mainnet datadir, seed or network address is used.
Executed on a disposable CI runner; --self-test also works in Termux.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]

TEST = ROOT / 'crates/nightfall-node/tests/ibd_short_pages.rs'
LOCK = ROOT / 'Cargo.lock'

TEST_NAME = 'real_node_recovers_every_short_wire_page'

MAX_BYTES = 1024 * 1024

PATTERN = re.compile(
    r'(?m)^test real_node_recovers_every_short_wire_page \.\.\. ok\s*$'
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def marker_ok(output):
    # Exit code alone is inadequate:
    # cargo can report zero executed tests.

    return len(
        PATTERN.findall(output.replace('\r', ''))
    ) == 1


def run_bounded(command, timeout, cap=MAX_BYTES):
    """Read output incrementally, limiting memory and process lifetime."""

    started = time.monotonic()

    proc = subprocess.Popen(
        command,
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
        env={
            **os.environ,
            'CARGO_TERM_COLOR': 'never',
            'NIGHTFALL_PEERS_URL': 'off',
        },
    )

    data = bytearray()
    deadline = started + timeout

    timeout_hit = False
    too_large = False

    try:
        with selectors.DefaultSelector() as sel:
            sel.register(proc.stdout, selectors.EVENT_READ)

            while sel.get_map():

                remaining = deadline - time.monotonic()

                if remaining <= 0:
                    timeout_hit = True
                    break

                for key, _ in sel.select(min(0.25, remaining)):

                    remaining_space = cap - len(data)

                    packet = os.read(
                        key.fileobj.fileno(),
                        min(65536, remaining_space + 1),
                    )

                    if not packet:
                        sel.unregister(key.fileobj)
                        continue

                    data.extend(packet[:remaining_space])

                    if len(packet) > remaining_space:
                        too_large = True
                        break

                if too_large:
                    break

        if not timeout_hit and not too_large:

            try:
                proc.wait(
                    timeout=max(
                        0,
                        deadline - time.monotonic()
                    )
                )

            except subprocess.TimeoutExpired:
                timeout_hit = True

    finally:

        if (
            timeout_hit
            or too_large
            or proc.poll() is None
        ):

            try:
                os.killpg(
                    proc.pid,
                    signal.SIGKILL,
                )

            except ProcessLookupError:
                pass

        proc.wait()
        proc.stdout.close()

    output = data.decode(
        'utf-8',
        errors='replace'
    )

    return {
        'exit_code': proc.returncode,
        'timed_out': timeout_hit,
        'output_limit_hit': too_large,
        'elapsed_ms': round(
            (time.monotonic() - started) * 1000
        ),
        'named_test_pass': marker_ok(output),
    }, output


def self_test():

    good = (
        'running 1 test\n'
        'test ' + TEST_NAME + ' ... ok\n'
    )

    assert marker_ok(good)

    assert not marker_ok(
        'running 0 tests\ntest result: ok\n'
    )

    assert not marker_ok(good + good)

    assert not marker_ok(
        good.replace('... ok', '... FAILED')
    )

    success, output = run_bounded(
        [
            sys.executable,
            '-c',
            'print("hello")',
        ],
        5,
        100,
    )

    assert success['exit_code'] == 0
    assert output == 'hello\n'

    failed, _ = run_bounded(
        [
            sys.executable,
            '-c',
            'import sys;sys.exit(4)',
        ],
        5,
        100,
    )

    assert failed['exit_code'] == 4

    flood, _ = run_bounded(
        [
            sys.executable,
            '-c',
            'print("Z" * 10000)',
        ],
        5,
        100,
    )

    assert flood['output_limit_hit']

    hang, _ = run_bounded(
        [
            sys.executable,
            '-c',
            'import time;time.sleep(5)',
        ],
        0.2,
        100,
    )

    assert hang['timed_out']

    print(
        'SELFTEST PASS: marker, exit, output cap, timeout'
    )


def main():

    parser = argparse.ArgumentParser()

    parser.add_argument(
        '--self-test',
        action='store_true'
    )

    parser.add_argument(
        '--repeats',
        type=int,
        default=3
    )

    parser.add_argument(
        '--timeout',
        type=int,
        default=180
    )

    parser.add_argument(
        '--report',
        type=Path
    )

    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    if args.repeats < 1 or args.repeats > 5:
        parser.error('repeats must be 1..5')

    if args.timeout < 30 or args.timeout > 900:
        parser.error('timeout must be 30..900 seconds')

    if not TEST.is_file() or not LOCK.is_file():
        parser.error('test or Cargo.lock missing')

    text = TEST.read_text(encoding='utf-8')

    if f'fn {TEST_NAME}()' not in text:
        parser.error(
            'expected real-node regression test missing'
        )

    commit = subprocess.check_output(
        [
            'git',
            'rev-parse',
            'HEAD',
        ],
        cwd=ROOT,
        text=True,
    ).strip()

    report = {
        'schema': 'nightfall-ibd-acceptance-v1',
        'scope': 'loopback-devnet-test-not-mainnet-proof',
        'commit': commit,
        'cargo_lock_sha256': sha(LOCK),
        'devnet_test_sha256': sha(TEST),
        'runs': [],
        'result': 'FAIL',
    }

    command = [
        'cargo',
        'test',
        '--locked',
        '-p',
        'nightfall-node',
        '--test',
        'ibd_short_pages',
        TEST_NAME,
        '--',
        '--exact',
        '--nocapture',
        '--test-threads=1',
    ]

    try:

        for count in range(1, args.repeats + 1):

            print(
                f'Devnet integration run {count}/{args.repeats}',
                flush=True,
            )

            result, output = run_bounded(
                command,
                args.timeout
            )

            result['run'] = count
            report['runs'].append(result)

            passed = (
                result['exit_code'] == 0
                and result['named_test_pass']
                and not result['timed_out']
                and not result['output_limit_hit']
            )

            if not passed:

                print(
                    'TEST FAILED (last 2500 output chars):',
                    file=sys.stderr,
                )

                print(
                    output[-2500:],
                    file=sys.stderr,
                )

                break

            print(
                f'PASS: {result["elapsed_ms"]} ms',
                flush=True,
            )

        else:
            report['result'] = 'PASS'

    finally:

        if args.report:

            args.report.parent.mkdir(
                parents=True,
                exist_ok=True
            )

            tmp = args.report.with_name(
                args.report.name + '.tmp'
            )

            tmp.write_text(
                json.dumps(
                    report,
                    indent=2,
                    sort_keys=True,
                ) + '\n',
                encoding='utf-8',
            )

            tmp.replace(args.report)

    print(
        'IBD GATE: ' + report['result'],
        flush=True,
    )

    return (
        0 if report['result'] == 'PASS'
        else 1
    )


if __name__ == '__main__':

    try:
        sys.exit(main())

    except (
        OSError,
        ValueError,
        subprocess.SubprocessError,
    ) as error:

        print(
            'IBD GATE ERROR: ' + str(error),
            file=sys.stderr,
        )

        sys.exit(2)
