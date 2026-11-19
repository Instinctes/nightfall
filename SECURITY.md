# Security policy

## Reporting a vulnerability

**Do not open a public issue for anything that affects consensus, the supply
invariant, or user funds.**

Email **instinctes@hotmail.com** with:

- what breaks, and the shortest path to reproducing it
- which crate and function
- a proof of concept if you have one — a failing test is ideal

You will get an acknowledgement within 72 hours. If you do not, assume the
address is broken and open a GitHub issue saying only *"contact me about a
security matter"* with no details.

## Bug bounty

Rewards are paid in NIGHT from a publicly listed address. **The reward amounts
and that address are not published yet.** The programme opens once they are
listed in this file. Until then, reports are welcome and will be credited, but
no reward is promised.

**Rules**

- Report privately, as above, and be the first to report the issue.
- Include enough to reproduce it. A failing test against `main` is ideal.
- Demonstrate on devnet, testnet or a private copy of the chain, never on
  mainnet. Do not spend, freeze or redirect anyone else's coins, and do not
  attack the seed nodes, the website or other people's infrastructure.
- Give us time to ship a fix: 90 days, or until a consensus fix has activated
  if one is needed. We will agree a disclosure date with you.
- No social engineering, phishing or physical attacks.

**Severity** (the categories below decide the tier):

| Tier | Examples |
|---|---|
| Critical | Inflation, theft, spending without the one-time key, key recovery, a consensus split between honest nodes |
| High | Destroying or redirecting other people's funds, a remote crash of nodes, recovering amounts or linking payments |
| Medium | Denial of service that needs real resources, bypassing coinbase maturity or fee rules |
| Low | Issues with limited impact that still need a fix |

**Safe harbour.** Research done in good faith and within these rules will not
be pursued by the project.

## What counts as critical

Anything in this list should be reported privately:

- **Inflation.** Any way to make `Σ UTXO − Σ kernel_excess = (minted − burned)·G` hold while creating value, or any way to mint outside the emission schedule.
- **Theft.** Spending an output you do not hold the one-time key for.
- **Fund destruction.** Making a payment unspendable by its recipient.
- **Consensus splits.** Two honest nodes accepting incompatible chains.
- **Remote crashes or resource exhaustion** triggerable by a peer.
- **Privacy breaks.** Recovering an amount, linking a payment to an address, or linking two payments to the same recipient.
- **Key recovery.** Anything that extracts a spend key from a view key, a signature, or on-chain data.

## What does not need to be private

Open a normal issue for UI bugs, documentation errors, build failures,
performance problems, and anything a user can see without it costing them
money.

## Known limitations — please do not report these

These are documented, deliberate, and already on the roadmap:

- **The transaction graph is linkable.** Block-level aggregation hides which
  input paid which output within a block, but spent outputs remain visible.
  Cut-through would remove the per-input signature that makes non-interactive
  payments safe. See [`docs/SPEC.md`](docs/SPEC.md) §7.
- **No network-layer privacy.** No Dandelion++. The first relaying node is
  probably the origin.
- **Initial sync takes time.** Memory-hard PoW makes verification expensive by
  design. Since 1.1.3, Fast start downloads the published chain archive and the
  node still verifies it; blocks after the newest checkpoint are validated in
  full. There are no UTXO snapshots yet.
- **Small network.** A young chain with little hashrate can be out-mined. This
  is a property of every new proof-of-work network, not a bug.

## Scope

The `crates/` tree, the wire protocol, and the release binaries.

The website is out of scope unless it serves a malicious binary — in which case
report it immediately, because it means the release pipeline was compromised.

## History

Protocol **v4 was consensus-broken**. Its balance proof was a tautology and
anyone could have minted unlimited coins; the recipient of every payment was
also published in cleartext. The full analysis, including working
proof-of-concept exploits, is in
[`docs/AUDIT-2026-08-12.md`](docs/AUDIT-2026-08-12.md).

That chain was discarded and v5 started from a fresh genesis. We publish our
own failures because a privacy project that hides them is not one you should
use.

## Disclosure

We will credit you unless you ask otherwise, publish a fix and an advisory
together, and describe the actual impact rather than downplaying it.

If a vulnerability is being actively exploited, we will say so publicly and
immediately, even before a fix exists. Users losing money quietly is worse than
users being warned.

## Advisories

- [November 2026 — transaction binding: pending payments could be redirected](docs/advisories/2026-11-transaction-binding.md),
  fixed at block 500,000 (1.1.1 and later).

## Current review

Internal (not independent): [docs/AUDIT-2026-08-16.md](docs/AUDIT-2026-08-16.md)

