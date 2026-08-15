# History of this chain

Nightfall has been reset more than once. That is not a secret. This file is
the list, so nobody has to reconstruct it from tags.

| Epoch | Genesis (prefix) | Why it ended |
|---|---|---|
| v4 (Nightproof-α) | discarded | Balance proof was a tautology. Anyone could mint. Full write-up: [AUDIT-2026-08-12.md](AUDIT-2026-08-12.md). |
| v5 / v6 | discarded | Sound cryptography, broken networking and wallet reorg handling. A payment could vanish after a fork with nothing reporting it. |
| **v7** (Aug 2026, ~a week) | `c8614333c0f86a4824df212474632f4b9feecf9bf0593841199d894127f2f9a6` | Emission too front-loaded (89 M of 90 M in 7 years). Two miners held most of the float. Privacy claims outran the code. |
| **v8 / n8** (this chain) | `061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de` | Live. 6 NIGHT / 7.5 M blocks. New magic `NFL2`, wire v6, datadir `nightfall/<net>/n8`. |

**Coins do not migrate.** A balance on `c8614333…` is a souvenir. It is not
NIGHT on this genesis. Software from 0.6.x cannot handshake with 0.7.0.

The v4 audit is kept because the bug class matters more than the chain it
killed. The v5 migration note is folded into this file: we did not credit
v4 balances into v5, and we do not credit v7 balances into v8, for the
same reason — a fair genesis cannot open with allocations whose provenance
is a discarded ledger.
