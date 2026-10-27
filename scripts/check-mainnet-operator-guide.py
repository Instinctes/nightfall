#!/usr/bin/env python3

"""
NIGHTFALL MAINNET OPERATOR INTEGRITY GUARD

Read-only verification of documentation against
the compiled protocol identity and published
genesis reference.

No wallet access.
No key handling.
No network connections.
No consensus modifications.
"""

from pathlib import Path
from tempfile import TemporaryDirectory
import re
import sys
import unittest


def issues(root: Path):

    types = (
        root / 'crates/nightfall-types/src/lib.rs'
    ).read_text()

    reset = (
        root / 'docs/RESET.md'
    ).read_text()

    readme = (
        root / 'README.md'
    ).read_text()

    guide = (
        root / 'docs/MAINNET.md'
    ).read_text()

    problems = []

    def number(name):

        match = re.search(
            r'pub const ' + name +
            r':\s*u32\s*=\s*(\d+)\s*;',
            types
        )

        if not match:
            problems.append(
                'Missing constant: ' + name
            )
            return None

        return int(match.group(1))

    protocol = number('PROTOCOL_VERSION')
    wire = number('WIRE_VERSION')

    match = re.search(
        r'^\|\s*Genesis\s*\|[^\n]*`([0-9a-f]{64})`',
        reset,
        re.M
    )

    if not match:
        problems.append(
            'Genesis reference unavailable.'
        )
        return problems

    genesis = match.group(1)

    if genesis[:8] not in readme:
        problems.append(
            'README genesis differs from RESET.'
        )

    if protocol is not None and wire is not None:

        expected = (
            f'**Protocol:** v{protocol} '
            f'(Nightproof) · **Wire:** v{wire}'
        )

        if expected not in guide:
            problems.append(
                'Guide protocol/wire version mismatch.'
            )

    if (
        f'**Genesis:** `{genesis}`'
        not in guide
    ):
        problems.append(
            'Guide genesis differs from RESET.'
        )

    if 'nightfall/mainnet/n8/' not in guide:
        problems.append(
            'Missing v8 datadir isolation.'
        )

    if (
        'heaviest valid chain by cumulative work'
        not in guide
    ):
        problems.append(
            'Missing cumulative-work explanation.'
        )

    forbidden = (
        'You are **Node Zero**',
        'Delete any pre-v7 datadir',
        'longest valid chain',
        '0.6.3 peers accept it',
        '**Every chain before v7 is dead.**'
    )

    for phrase in forbidden:

        if phrase in guide:
            problems.append(
                'Outdated instruction: ' + phrase
            )

    if 'Do not mine without peers' not in guide:
        problems.append(
            'Missing mining isolation warning.'
        )

    return problems


class IntegrityTests(unittest.TestCase):

    def test_current_repository(self):

        root = (
            Path(__file__).resolve().parents[1]
        )

        self.assertEqual(
            [],
            issues(root)
        )

    def test_detects_documentation_regressions(self):

        root = (
            Path(__file__).resolve().parents[1]
        )

        with TemporaryDirectory() as tmp:

            target = Path(tmp)

            files = [
                'crates/nightfall-types/src/lib.rs',
                'docs/RESET.md',
                'README.md',
                'docs/MAINNET.md'
            ]

            for name in files:

                destination = target / name

                destination.parent.mkdir(
                    parents=True,
                    exist_ok=True
                )

                destination.write_text(
                    (root / name).read_text()
                )

            guide = target / 'docs/MAINNET.md'

            original = guide.read_text()

            guide.write_text(
                original.replace(
                    '**Wire:** v6',
                    '**Wire:** v4'
                )
            )

            self.assertTrue(
                any(
                    'protocol/wire' in error
                    for error in issues(target)
                )
            )

            guide.write_text(
                original.replace(
                    'heaviest valid chain by cumulative work',
                    'longest valid chain'
                )
            )

            self.assertTrue(
                any(
                    'cumulative-work' in error
                    for error in issues(target)
                )
            )

            old_genesis = (
                '061a052d49607ff8f4b306c75d622eb'
                'd230cff4ec3a45a6dffc2f7738d4b20de'
            )

            guide.write_text(
                original.replace(
                    old_genesis,
                    'f' * 64
                )
            )

            self.assertTrue(
                any(
                    'genesis differs' in error
                    for error in issues(target)
                )
            )


if __name__ == '__main__':

    if '--test' in sys.argv:

        sys.argv.remove('--test')

        unittest.main()

    else:

        root = (
            Path(__file__).resolve().parents[1]
        )

        errors = issues(root)

        if errors:

            for error in errors:
                print(
                    'FAIL:',
                    error,
                    file=sys.stderr
                )

            sys.exit(1)

        print(
            'PASS: Mainnet operator documentation '
            'matches protocol identity.'
        )
