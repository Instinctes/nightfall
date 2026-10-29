# Nightpool: Offline Work Accounting Research

Status: Research only.

This is not a mining pool, network implementation,
consensus verifier or wallet.

The October 2026 Nightfall roadmap identifies
operator-free pool mining as high-priority research.

## Goal

Evaluate whether a bounded work-weighted PPLNS
window produces deterministic and exact integer
payouts from a claimed share history.

## Current capabilities

- Bounded input parsing
- Duplicate share ID rejection
- Linear ancestry checks
- Integer work accounting
- Partial oldest-share accounting
- Deterministic rounding
- Exact reward conservation
- Reproducible offline examples
- Adversarial regression tests

The reference illustrates one era-0 reward of
600000000 darks.

This is not a proposal to change emission.

## Run

python3 research/nightpool/payout_reference.py --demo

python3 -m unittest discover \
  -s research/nightpool \
  -p 'test_*.py' -v

## Important limitations

Share work is asserted by the caller.

No actual Nightfall proof of work is validated.

A share ID is not authenticated.

No candidate block is validated.

The model does not solve selfish mining,
share withholding, Sybil attacks, eclipse attacks,
network partitions or competing share branches.

No coins are paid.

No noncustodial payout enforcement exists.

No live miner, node, wallet or mainnet data is used.

Input checksums serve reproducibility only.
They do not authenticate work.

## Next engineering gates

1. Versioned share header tied to Nightfall genesis.

2. Independent validation of actual share PoW.

3. Work-based fork selection and replay rules.

4. Explicit handling of competing shares,
   network partitions, withholding and reorgs.

5. Enforceable noncustodial coinbase payouts
   compatible with Nightfall consensus.

6. Multi-node adversarial Devnet tests.

7. Independent security review before deployment.

A complete Devnet share chain is not implemented.

Protocol version, wire version, genesis, emission,
fees and consensus remain unchanged.

No real recovery words, miner seeds, wallet
addresses or private network details belong in
research input files.
