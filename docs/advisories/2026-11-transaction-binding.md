# Transaction binding: pending payments could be redirected (fixed at block 500,000)

**Status:** fixed. The binding rules have been active on mainnet since block
500,000 (11 November 2026, 21:45 UTC).
**Affected:** every node and wallet before 1.1.1, and transactions on the chain below block 500,000.
**Fixed in:** 1.1.1 (rules activate at 500,000). Current release: 1.1.4.
**Supply:** never affected. The supply equation held for every block, and no NIGHT was created that should not exist.

## Summary

An internal security review in late October 2026 found that a NIGHTFALLCOIN
transaction did not bind its inputs and outputs tightly enough to each other.
Someone who already knew certain secrets of a payment could take a payment
that was still waiting to be mined and build a different transaction from it:

- **Redirecting an outgoing payment (critical).** The sender of a payment, or
  anyone holding the *payer's* view key, could take the payer's waiting
  transaction and send the coins somewhere else.
- **Taking an incoming payment (critical).** Anyone holding the *recipient's*
  view key could replace the recipient's output in a waiting transaction with
  their own.

Neither attack worked against a payment that was already in a block, and
neither let anyone spend coins they had never been involved with. They
required knowledge that a stranger does not have: being the sender, or having
been given a view key.

The same review found several smaller issues, fixed in the same upgrade:

| Finding | Severity | Effect |
|---|---|---|
| The mempool kept conflicting spends and the block builder preferred the higher fee | High | Made the redirect above work without a miner's help |
| One transaction with a coinbase-flagged output could make nodes build empty blocks | High (denial of service) | Transactions stopped confirming for hours, no funds at risk |
| Coinbase maturity could be bypassed by a miner | Medium | A miner could spend new coins before the 1,440-block wait |
| Kernel fees were summed without overflow checks | Low | Undefined accounting; no inflation was possible |
| Transaction boundaries inside a block were recoverable | Privacy | Weaker transaction hiding than the documentation claimed |

## What changed

From block 500,000 every input carries a fresh key and a signature that only
the real owner can produce, and every kernel carries an extra key. One
equation over the whole block forces these keys to balance. A transaction
can no longer be rebuilt by someone who does not know all of its keys. A
block-wide kernel offset hides transaction boundaries. The coinbase output
must open to exactly the subsidy, and fees are bounded and summed with
checks. The details are in `docs/SPEC.md` §1.6.

The mempool changes (first seen wins, no coinbase-flagged outputs) applied as
soon as a node ran 1.1.1.

## Was anyone affected?

We checked what can be checked:

- **Supply:** every node verifies the supply equation for every block. It
  held throughout, before and after the fork.
- **Coinbase maturity:** we scanned all 500,000 blocks before the fork. In
  every one of them the coinbase-flagged output opens to exactly the block
  subsidy. The maturity bypass was never used.
- **Fees:** no kernel in those blocks has an out-of-range fee, and no
  block's fee sum overflows.
- **Redirected payments:** a redirected payment looks like an ordinary
  transaction on the chain, so the chain alone cannot prove that it never
  happened. Nobody has reported a lost or redirected payment.

## What you should do

- Run **1.1.1 or later**; the current release is 1.1.4. Nodes still on 1.0.x
  stopped following the network at block 500,000 and do not see new blocks.
- If you ever shared a **view key** before 11 November 2026, it could only be
  misused against payments that were still waiting at the time. Those
  payments are long since mined or gone. Today a view key reveals your
  incoming payments, as documented, but it can no longer move anything.

## Timeline

| Date (UTC) | |
|---|---|
| 29 Oct 2026 | Findings confirmed in an internal review; fix written and tested against a copy of the real chain |
| 29 Oct 2026 | 1.1.1 released: new rules activating at block 500,000, mempool hardening immediately |
| 29 Oct – 11 Nov | Seeds and pool operators upgrade |
| 11 Nov 2026 21:45 | Block 500,000: the binding rules are active |
| 19 Nov 2026 | This disclosure |

The details of the attacks were kept private until the fork had activated and
the network had moved past it.

## Reporting

Please report security issues privately, as described in `SECURITY.md`.
Thank you to the pool operators and node runners who upgraded before block
500,000.
