# Funded recovery assurance

## Purpose

Verify that recovery restores actual spending capability,
rather than only reproducing a receiving address.

This complements existing recovery validation, vault tests,
scan tests and multi-node recovery work.

## Reproduction

Run from the repository root:

    cargo test --locked -p nightfall-wallet \
      --test funded_recovery_assurance \
      -- --test-threads=1

## Acceptance criteria

- Restore a funded Devnet wallet from recovery words.
- Restore a stale encrypted wallet snapshot.
- Rescan a canonical, locally generated Devnet chain.
- Compare recovered identity, scan anchor and owned outputs.
- Compare spendable funds against the original wallet.
- Reject recovery of those funds with unrelated valid words.
- Build and validate a payment using recovered keys.
- Include the payment in a locally mined block.
- Confirm the recipient receives the expected amount.
- Reject a subsequent attempt to reuse consumed inputs.
- Recheck the monetary supply invariant.

## Trust and privacy boundaries

The fixtures use public synthetic keys and passwords.
No production wallet, real funds, RPC service or public peer
is required. No runtime dependency is added.

These tests do not establish the security of Android storage,
the GUI import workflow, a remote light node, physical backup
media or the mainnet implementation against all adversaries.

Passing this test is not an independent cryptographic audit.

No consensus, emission, mining or wallet-format change.
