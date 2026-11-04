# NIGHTFALLCOIN 1.1.0 — consensus upgrade at block 500,000

1.1.0 is a **required upgrade** for every node, miner, pool and wallet. It
activates a consensus upgrade at **block 500,000**. Below that height nothing
changes: blocks are validated, hashed and stored exactly as in 1.0.x. From
block 500,000 on, nodes running 1.0.x no longer follow the chain.

Same chain, same genesis, same coins. Supply, emission, block reward, block
time and proof of work are unchanged. There is nothing to migrate.

## Consensus upgrade (from block 500,000)

- **Transaction binding.** Every input carries a fresh input key with its own
  signature, every kernel a stealth excess, and one block-wide equation
  balances them against the outputs' ephemeral keys. The construction follows
  Litecoin MWEB's stealth offsets. See `docs/SPEC.md` §1.6.
- **Kernel offsets.** A kernel no longer matches the exact set of commitments
  of its own transaction, which makes the transactions in a block much harder
  to tell apart.
- **Coinbase opening.** The coinbase-flagged output must hold exactly the
  coinbase amount.
- **Fee bounds.** Every kernel fee is bounded and fee sums are checked.

Wallets built with 1.1.0 attach the new fields from today, so a payment sent
before block 500,000 stays valid after it.

## Node and mempool

- A transaction that spends an input already held by another pending
  transaction is refused, whatever its fee. First seen wins.
- Transactions carrying a coinbase-flagged output are refused at admission
  and dropped from block templates.

## New in Core

- **Nightfall Pay**: payment requests and invoices you can share as links.
- **My Node**: serve the web wallet from your own domain, talking to your own node.
- **Node Kit** (`nightfall-nodekit`): set up and check your own node without root and without touching a running node's data.
- **Failed payments** stay in your history, marked as not completed, and their
  coins are released instead of staying reserved.

## Upgrading

Close Core completely before replacing the app. Keep your 24 words and an
encrypted backup, and never delete your data folder to update. Node operators
and pools: upgrade before block 500,000. A node that is not upgraded stops
following the network after the activation height and needs 1.1.0 to rejoin.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are
built by GitHub Actions with `--locked`, gated by platform tests, and published
with SHA256 manifests and GitHub build provenance. Verify with
`scripts/check_release_provenance.py` or `gh attestation verify <file> --repo
Instinctes/nightfall`. Provenance is not Apple notarisation, Windows
Authenticode signing or an independent security audit.
