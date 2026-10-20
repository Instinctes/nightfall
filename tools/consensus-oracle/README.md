# Nightfall Independent Consensus Oracle

Standalone, read-only validation suite for Nightfall Mainnet.

The goal is to reduce common-mode implementation risk by independently reconstructing consensus, proof-of-work, block-body, cryptographic and ledger-state properties without importing Nightfall production implementations.

The Oracle is intentionally maintained as a separate Cargo package.

## Scope

### Phase 1 — public header-chain validation

`nightfall-consensus-oracle` walks every available public Mainnet header through the observed tip and independently checks network/protocol/genesis consistency, parent and height linkage, emission, minted supply, maximum supply, LWMA difficulty, median-time-past, cumulative work, checkpoint 25,000 and final tip/status consistency.

### Phase 2A — canonical block hashing

`p2p-hash-probe` independently reconstructs domain-separated BLAKE3 header encoding, the PoW preimage, nonce binding, canonical block hashes and parent linkage from full P2P blocks.

### Phase 2B — Nighthash-v2 PoW

`p2p-pow-probe` independently verifies sampled Mainnet proof of work with Argon2id v=0x13, 32,768 KiB memory, one iteration, one lane and independent arbitrary-precision difficulty arithmetic.

Default sampling covers genesis, the checkpoint region and the observed tip.

### Phase 2C-1 — block-body validation

`p2p-body-probe` independently checks body-root reconstruction, canonical ordering, aggregate limits, duplicate inputs/outputs, coinbase structure, reward schedule and coinbase lock height.

### Phase 2C-2 — transfer cryptography

`p2p-transfer-crypto-probe` locates real Mainnet transfer blocks and verifies Bulletproof range proofs, output Schnorr signatures, kernel Schnorr signatures, body anchoring and Pedersen balance equations using primitive libraries directly.

### Phase 2C-3 — state replay

`p2p-state-replay-probe` replays Mainnet from genesis through height 2,326 and independently checks UTXO existence, duplicate/unknown spends, coinbase maturity, input ownership signatures, UTXO transitions, UTXO roots, cumulative kernel sums and canonical block anchoring.

A deliberately corrupted input signature is used as a negative control and must be rejected.

### Phase 2D — adversarial property model

`phase2d-adversarial-probe` exercises maturity boundaries, duplicate-spend semantics, UTXO Merkle mutations, header/nonce binding, Schnorr mutations and Pedersen one-unit inflation mutations.

## Independence boundary

The Oracle source does not import Nightfall production consensus, ledger, crypto, P2P, storage, node, wallet or types implementations.

It does use third-party cryptographic primitive implementations including BLAKE3, Argon2, curve25519-dalek, Merlin and Bulletproofs.

This is an independently composed validation implementation, not a from-scratch implementation of those cryptographic primitives.

## Production boundary

This contribution is additive. It does not intentionally modify Nightfall consensus rules, ledger rules, block/transaction formats, P2P wire format, protocol version, genesis, emission, node runtime, wallet behavior or Mainnet state.

## Build

Run from the repository root:

    cargo fmt --manifest-path tools/consensus-oracle/Cargo.toml -- --check
    cargo clippy --manifest-path tools/consensus-oracle/Cargo.toml --all-targets -- -D warnings
    cargo test --manifest-path tools/consensus-oracle/Cargo.toml
    cargo build --release --manifest-path tools/consensus-oracle/Cargo.toml --bins

## Runtime

Phase 1 requires the public Mainnet light API. P2P probes require a reachable Mainnet archive peer.

Some probes intentionally use sampling. The state replay currently covers heights 0 through 2,326.

A successful run means the explicitly listed properties matched the observed Mainnet data for the tested scope. It is not formal verification, proof of absence of bugs, or an independent professional security audit.
