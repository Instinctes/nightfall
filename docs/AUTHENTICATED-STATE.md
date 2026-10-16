# Authenticated UTXO state — experimental

Status: research / shadow implementation only.

This work MUST NOT create a new genesis merely to activate a new
state accumulator.

## Economic continuity invariant

For any future same-chain activation at height H:

1. Every UTXO valid immediately before H remains the same UTXO.
2. The same one-time spending key continues to authorize it.
3. No balance is reminted.
4. No balance is redistributed.
5. No founder/team/upgrade allocation is introduced.
6. Minted and burned supply counters are preserved exactly.
7. The new authenticated-state root is derived from the already
   validated UTXO state.
8. Failure to derive the new root aborts activation rather than
   silently constructing a replacement state.

Therefore the state-structure upgrade itself does not invalidate
previously mined NIGHT.

## Stage 1

`nightfall-ledger::authenticated_state` is deliberately independent
of protocol-v8 consensus.

It provides:

- 256-bit sparse Merkle state keyed by UTXO commitment
- incremental insert/delete path updates
- canonical empty subtrees
- membership proofs
- authentication of commitment, output key, creation height and
  coinbase status

The current `UtxoSet::root()` remains authoritative.

## Required before any consensus proposal

- shadow-run against validated chain state
- deterministic rebuild tests
- randomized insert/spend/reorg property tests
- persisted versioned generations
- crash tests at every persistence boundary
- rollback/reorg tests
- benchmark against current UTXO root
- independent review
- explicit same-chain activation specification

No protocol-version or genesis change belongs in Stage 1.

## Stage 2 — atomic shadow transitions

Stage 2 adds a non-consensus transition engine.

After the canonical ledger accepts a block, the experimental tree can
independently apply the same spends and creations.

A shadow transition succeeds only if:

1. every spent commitment existed,
2. no created commitment replaced an existing leaf,
3. the resulting complete leaf map equals the canonical UTXO map,
4. the incremental authenticated root equals an independently rebuilt root.

The operation is transactional. A failed check leaves the prior shadow state
unchanged.

This still does not modify `LedgerState`, block validation, protocol version,
genesis, emission, wallet ownership, or mainnet consensus.

## Stage 3 — delta rollback journal

The shadow transition no longer clones the complete authenticated tree before
every block.

Instead it records a minimal undo journal containing only UTXOs actually
removed or created by the candidate shadow transition.

If any post-transition invariant fails, journal entries are replayed in reverse
order. Tests require rollback to restore:

- the exact canonical leaf map,
- the exact sparse-node map,
- the exact authenticated root.

The independent full rebuild remains enabled as a development oracle. It is
not intended to remain mandatory in the production hot path.

This remains experimental shadow infrastructure and is not consensus-active.
