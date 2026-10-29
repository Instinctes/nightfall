#!/usr/bin/env python3

from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]

source = (
    root
    / 'crates'
    / 'nightfall-types'
    / 'src'
    / 'lib.rs'
)

text = source.read_text(encoding='utf-8')

expected = {
    'PROTOCOL_VERSION': 8,
    'WIRE_VERSION': 6,
    'MAX_SUPPLY_NIGHT': 90000000,
    'TARGET_BLOCK_TIME_SECS': 15,
}

for key, number in expected.items():
    pattern = (
        r'^pub const '
        + re.escape(key)
        + r':\s*u(?:32|64)\s*=\s*([\d_]+)\s*;'
    )

    found = re.findall(
        pattern,
        text,
        re.MULTILINE,
    )

    if len(found) != 1:
        raise SystemExit(
            f'FAIL: missing or duplicate {key}'
        )

    actual = int(
        found[0].replace('_', '')
    )

    if actual != number:
        raise SystemExit(
            f'FAIL: unexpected {key}'
        )

print('PASS: protected v8 network constants')
