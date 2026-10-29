#!/usr/bin/env python3
"""Fail closed when experimental branches change mainnet identity constants."""
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parent.parent
file = root / 'crates/nightfall-types/src/lib.rs'
text = file.read_text(encoding='utf-8')
values = {'PROTOCOL_VERSION': 8, 'WIRE_VERSION': 6,
          'MAX_SUPPLY_NIGHT': 90_000_000, 'TARGET_BLOCK_TIME_SECS': 15}
for name, expected in values.items():
    found = re.findall(r'^pub const ' + re.escape(name) +
                       r':\s*u(?:32|64)\s*=\s*([\d_]+)\s*;', text, re.M)
    if len(found) != 1 or int(found[0].replace('_', '')) != expected:
        sys.exit(f'FAIL: unexpected mainnet constant {name}')
print('PASS: v8/wire6/supply/target-time constants unchanged')
