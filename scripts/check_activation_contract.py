#!/usr/bin/env python3
"""Source identity guard, not a consensus audit."""

import argparse
import hashlib
import json
import re

from pathlib import Path

FILES = [
    'crates/nightfall-types/src/lib.rs',
    'crates/nightfall-storage/src/codec.rs',
    'crates/nightfall-consensus/tests/binding_chain.rs',
    'docs/SPEC.md',
    'docs/RESET.md',
    'RELEASE-NOTES/RELEASE-NOTES-v1.1.1.md',
]

GENESIS = (
    '061a052d49607ff8f4b306c75d622eb'
    'd230cff4ec3a45a6dffc2f7738d4b20de'
)

VALUES = {
    'PROTOCOL_VERSION': 8,
    'WIRE_VERSION': 6,
    'BINDING_FORK_HEIGHT_MAINNET': 500000,
    'DARKS_PER_NIGHT': 100000000,
    'MAX_SUPPLY_NIGHT': 90000000,
}


def verify(s):
    t, c, b, spec, reset, notes = (
        s[f] for f in FILES
    )

    bad = []

    def need(ok, reason):
        if not ok:
            bad.append(reason)

    for name, val in VALUES.items():
        m = re.search(
            r'\bpub const ' + name +
            r':\s*u(?:32|64)\s*=\s*([\d_]+)\s*;',
            t,
        )

        need(
            m is not None and
            int(m.group(1).replace('_', '')) == val,
            'constant ' + name,
        )

    fork = t.partition(
        'pub fn binding_fork_height(self) -> u64 {'
    )[2]

    fork = fork.partition(
        'pub fn default_p2p_port'
    )[0]

    need(
        bool(re.search(
            r'Self::Mainnet\s*=>\s*'
            r'BINDING_FORK_HEIGHT_MAINNET',
            fork,
        )),
        'mainnet fork routing',
    )

    need(
        bool(re.search(
            r'Self::Devnet\s*=>\s*20\b',
            fork,
        )),
        'devnet routing',
    )

    need(
        GENESIS in spec and GENESIS in reset,
        'v8 genesis identity',
    )

    for item in (
        '**Protocol version:** 8',
        '**Wire version:** 6',
        '500,000',
    ):
        need(item in spec, 'SPEC: ' + item)

    for item in (
        '1.1.1',
        '500,000',
        '1.0.x',
        'Same chain, same genesis, same coins',
    ):
        need(
            item in notes,
            'release notes: ' + item,
        )

    for item in (
        'RECORD_V2_MAGIC',
        'decode_binary_record',
        'strip_prefix(&RECORD_V2_MAGIC)',
        'legacy::Block::from_current',
        'block.body.has_any_binding()',
        'records_switch_layout_exactly_at_the_binding_fork',
    ):
        need(
            item in c,
            'storage compatibility: ' + item,
        )

    for item in (
        'a_chain_crosses_the_fork_and_replays',
        'after_the_fork_an_unbound_transaction_is_refused',
        'chain.verify_supply().unwrap()',
    ):
        need(
            item in b,
            'consensus test fixture: ' + item,
        )

    return bad


def selftest(s):
    assert not verify(s), (
        'Baseline failed: ' + str(verify(s))
    )

    changes = [
        (
            0,
            'BINDING_FORK_HEIGHT_MAINNET: u64 = 500_000;',
            'BINDING_FORK_HEIGHT_MAINNET: u64 = 500_001;',
        ),
        (
            0,
            'PROTOCOL_VERSION: u32 = 8;',
            'PROTOCOL_VERSION: u32 = 9;',
        ),
        (
            0,
            'Self::Mainnet => BINDING_FORK_HEIGHT_MAINNET',
            'Self::Mainnet => 500_001',
        ),
        (
            4,
            GENESIS,
            '0' * 64,
        ),
        (
            1,
            'strip_prefix(&RECORD_V2_MAGIC)',
            'strip_prefix(&OTHER_MAGIC)',
        ),
        (
            2,
            'after_the_fork_an_unbound_transaction_is_refused',
            'REMOVED_TEST',
        ),
        (
            5,
            'Same chain, same genesis, same coins',
            'DIFFERENT CHAIN',
        ),
    ]

    for index, old, new in changes:
        f = FILES[index]

        assert old in s[f], (
            'Fixture drift: ' + f
        )

        corrupt = dict(s)

        corrupt[f] = s[f].replace(
            old,
            new,
            1,
        )

        assert verify(corrupt), (
            'Undetected mutation: ' + f
        )

    print(
        'PASS: baseline plus 7 mutation tests (8/8)'
    )


def main():
    ap = argparse.ArgumentParser()

    ap.add_argument(
        '--selftest',
        action='store_true',
    )

    ap.add_argument(
        '--json',
        type=Path,
    )

    a = ap.parse_args()

    root = Path(__file__).resolve().parents[1]

    s = {
        f: (root / f).read_text(encoding='utf-8')
        for f in FILES
    }

    errors = verify(s)

    if a.selftest:
        selftest(s)

    if a.json:
        a.json.write_text(
            json.dumps(
                {
                    'result': (
                        'FAIL' if errors else 'PASS'
                    ),
                    'scope': (
                        'source identity only; '
                        'no Rust execution'
                    ),
                    'files': {
                        f: hashlib.sha256(
                            s[f].encode()
                        ).hexdigest()
                        for f in FILES
                    },
                    'errors': errors,
                },
                indent=2,
            ) + '\n',
            encoding='utf-8',
        )

    for e in errors:
        print('FAIL:', e)

    if errors:
        raise SystemExit(1)

    print(
        'PASS: release identity '
        '+ compatibility fixtures'
    )


if __name__ == '__main__':
    main()
