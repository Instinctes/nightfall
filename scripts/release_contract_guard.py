#!/usr/bin/env python3
"""Read-only Nightfall v8 release contract guard."""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

GENESIS = (
    '061a052d49607ff8f4b306c75d622eb'
    'd230cff4ec3a45a6dffc2f7738d4b20de'
)

EXPECTED = {
    'PROTOCOL_VERSION': 8,
    'WIRE_VERSION': 6,
    'BINDING_FORK_HEIGHT_MAINNET': 500000,
    'DARKS_PER_NIGHT': 100000000,
    'MAX_SUPPLY_NIGHT': 90000000,
    'INITIAL_BLOCK_REWARD_NIGHT': 6,
    'HALVING_INTERVAL_BLOCKS': 7500000
}

def check(root=ROOT):

    files = (
        'crates/nightfall-types/src/lib.rs',
        'crates/nightfall-consensus/src/lib.rs',
        'docs/SPEC.md',
        'docs/RESET.md',
        'README.md'
    )

    content = {}
    failures = []

    for name in files:
        try:
            content[name] = (
                (root / name).read_text(
                    encoding='utf-8'
                )
            )
        except (OSError, UnicodeError) as exc:
            failures.append(f'{name}: {exc}')

    if failures:
        return failures

    types, consensus, spec, reset, readme = (
        content[name] for name in files
    )

    def require(name, condition):
        if not condition:
            failures.append(name)

    for name, expected in EXPECTED.items():

        match = re.search(
            r'(?m)^pub\s+const\s+' + name +
            r'\s*:\s*\w+\s*=\s*([0-9_]+)\s*;',
            types
        )

        require(
            'constant: ' + name,
            bool(match) and
            int(
                match.group(1).replace('_', '')
            ) == expected
        )

    require(
        'genesis in specification',
        GENESIS in spec
    )

    require(
        'genesis in reset',
        GENESIS in reset
    )

    require(
        'spec protocol/wire',
        bool(
            re.search(
                r'Protocol version:\*\*\s*8'
                r'.*Wire version:\*\*\s*6',
                spec
            )
        )
    )

    require(
        'mainnet fork dispatch',
        bool(
            re.search(
                r'Self::Mainnet\s*=>\s*'
                r'BINDING_FORK_HEIGHT_MAINNET',
                types
            )
        )
    )

    require(
        'testnet fork dispatch',
        bool(
            re.search(
                r'Self::Testnet\s*=>\s*0\s*,',
                types
            )
        )
    )

    require(
        'devnet fork dispatch',
        bool(
            re.search(
                r'Self::Devnet\s*=>\s*20\s*,',
                types
            )
        )
    )

    require(
        'activation in specification',
        bool(
            re.search(
                r'(?m)^### 1\.6[^\n]*500[,_ ]?000',
                spec
            )
        )
    )

    require(
        'activation in README',
        bool(
            re.search(
                r'500[,_ ]?000',
                readme
            )
        )
    )

    notes = root / 'RELEASE-NOTES'

    release_files = list(
        notes.glob(
            'RELEASE-NOTES-v1.1.*.md'
        )
    )

    require(
        'activation in release notes',
        bool(release_files) and any(
            re.search(
                r'500[,_ ]?000',
                f.read_text(encoding='utf-8')
            )
            for f in release_files
        )
    )

    require(
        'supply constant relationship',
        bool(
            re.search(
                r'MAX_SUPPLY_DARKS\s*:\s*u64'
                r'\s*=\s*MAX_SUPPLY_NIGHT'
                r'\s*\*\s*DARKS_PER_NIGHT',
                types
            )
        )
    )

    rules = (
        (
            'halving division',
            r'height\.0\s*/\s*self\.halving_interval'
        ),
        (
            'halving reward shift',
            r'self\.initial_reward_darks'
            r'\s*>>\s*halvings'
        ),
        (
            'supply clamp',
            r'theoretical\.min\(room\)'
        ),
        (
            'remaining room',
            r'self\.max_supply_darks'
            r'\s*-\s*total_minted_darks'
        )
    )

    for name, regex in rules:
        require(
            name,
            bool(re.search(regex, consensus))
        )

    initial = (
        EXPECTED['INITIAL_BLOCK_REWARD_NIGHT']
        * EXPECTED['DARKS_PER_NIGHT']
    )

    period = EXPECTED[
        'HALVING_INTERVAL_BLOCKS'
    ]

    emitted = sum(
        (initial >> i) * period
        for i in range(64)
    )

    require(
        'theoretical terminal supply',
        emitted == 8999999925000000
    )

    require(
        'terminal supply under ceiling',
        emitted <= (
            EXPECTED['MAX_SUPPLY_NIGHT']
            * EXPECTED['DARKS_PER_NIGHT']
        )
    )

    require(
        'halving boundary',
        (
            initial >> ((period - 1) // period),
            initial >> (period // period)
        ) == (
            initial,
            initial // 2
        )
    )

    return failures


def test():

    from tempfile import TemporaryDirectory
    from shutil import copy2

    files = (
        'crates/nightfall-types/src/lib.rs',
        'crates/nightfall-consensus/src/lib.rs',
        'docs/SPEC.md',
        'docs/RESET.md',
        'README.md'
    )

    notes = list(
        (ROOT / 'RELEASE-NOTES').glob(
            'RELEASE-NOTES-v1.1.*.md'
        )
    )

    assert notes, 'No 1.1 release notes'

    with TemporaryDirectory() as tmp:

        dest = Path(tmp)

        for source in (
            [ROOT / x for x in files] + notes
        ):

            target = dest / source.relative_to(ROOT)

            target.parent.mkdir(
                parents=True,
                exist_ok=True
            )

            copy2(source, target)

        baseline = check(dest)

        if baseline:
            raise RuntimeError(
                'Baseline failed: '
                + ', '.join(baseline)
            )

        cases = (
            (
                'crates/nightfall-types/src/lib.rs',
                '500_000',
                '500_001'
            ),
            (
                'crates/nightfall-types/src/lib.rs',
                '90_000_000',
                '91_000_000'
            ),
            (
                'crates/nightfall-consensus/src/lib.rs',
                'theoretical.min(room)',
                'theoretical'
            ),
            (
                'docs/RESET.md',
                GENESIS,
                '0' * 64
            ),
            (
                'docs/SPEC.md',
                '500,000',
                '510,000'
            )
        )

        for filename, old, new in cases:

            target = dest / filename

            before = target.read_text(
                encoding='utf-8'
            )

            if old not in before:
                raise RuntimeError(
                    'Mutation anchor missing: '
                    + filename
                )

            target.write_text(
                before.replace(old, new, 1),
                encoding='utf-8'
            )

            detected = check(dest)

            target.write_text(
                before,
                encoding='utf-8'
            )

            if not detected:
                raise RuntimeError(
                    'Fail-open regression: '
                    + filename
                )

    print('PASS: 5 negative regressions')


if __name__ == '__main__':

    parser = argparse.ArgumentParser()

    parser.add_argument(
        '--test',
        action='store_true'
    )

    args = parser.parse_args()

    if args.test:
        test()

    failures = check()

    for failure in failures:
        print(
            'FAIL:',
            failure,
            file=sys.stderr
        )

    print(
        'Release contract:',
        'PASS' if not failures else 'FAIL'
    )

    sys.exit(bool(failures))
