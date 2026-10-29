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

## Stage 4 — chain-level shadow integration

The authenticated UTXO tree is now integrated into the real `Chain` state as
non-consensus shadow state.

Protocol-v8 consensus remains unchanged:

- `BlockHeader::utxo_root` remains authoritative,
- `LedgerState` remains canonical,
- the authenticated shadow root is not serialized into block headers,
- no wire format changes are introduced,
- no protocol-version change is introduced,
- no genesis change is introduced,
- no emission or ownership rule changes are introduced.

### Normal block acceptance

The current upstream consensus uses in-place canonical ledger
validation together with a `StateUndo` rollback record.

Only after the existing canonical block, UTXO, kernel and
supply checks have passed is the transition mirrored into
the experimental authenticated UTXO tree.

The shadow transition checks its leaf map against the
canonical UTXO set and compares its incremental root
against an independent rebuild.

If the shadow transition fails, its own delta journal
restores the prior shadow state and the canonical ledger
is rewound with `StateUndo`.

Chain tip, accumulated work, headers, block history and
the undo window are updated only after shadow success.

The experimental shadow remains outside consensus data.
Its fail-closed availability implications require review.

### Trusted local replay

`apply_block_from_own_disk` retains upstream binding-field
normalization and canonical state-only replay.

It captures canonical undo state before replay and mirrors
the resulting UTXO transition into the shadow.

On a shadow failure, the canonical ledger is rewound.
The chain tip, accumulated work, block history and undo
window must remain unchanged.

Trusted replay is not a substitute for validating
untrusted peer blocks.

### Pruned-chain reconstruction

A pruned rebuild does not begin at genesis. It begins from the materialized
canonical pruning horizon.

The authenticated shadow is therefore reconstructed directly from that exact
horizon UTXO set before the retained suffix is replayed.

The same rule applies when a pruned datadir is restored after restart. Storage
first loads the validated canonical horizon, explicitly rebuilds the
non-consensus authenticated shadow from that canonical UTXO state, verifies
their equality, and only then replays retained block bodies.

This explicit restore boundary is not used during ordinary block acceptance.
A runtime shadow mismatch remains an error and is never silently repaired.

Tests require a successful pruned reorg to converge to the same:

- tip,
- canonical UTXO root,
- kernel sum,
- authenticated UTXO root

as the corresponding complete winning chain.

### Reorg isolation

Reorg candidates continue to be constructed independently from the active
chain before adoption.

Fault-injection coverage verifies that an invalid candidate can process a
sequence of valid blocks and then fail late in its untrusted suffix without
modifying the active pruned chain.

The test snapshots and compares:

- tip hash,
- cumulative work,
- retained block history,
- compact-header history,
- prune boundary,
- pruning horizon,
- canonical UTXO root,
- kernel sum,
- ledger height,
- transaction count,
- minted supply,
- burned supply,
- authenticated shadow root.

### Visibility

`AuthenticatedUtxoTree` remains an internal `Chain` implementation detail.

The public diagnostic surface exposes only the authenticated root and an
explicit deep consistency check. External callers cannot directly mutate the
shadow state.

### Current performance status

The integration is intentionally correctness-first.

The transition engine uses its delta undo journal rather than cloning the
complete authenticated tree for every block. However, exact comparison against
the canonical UTXO map and an independent clean rebuild are still enabled as
development correctness oracles.

Those full checks are not intended to remain mandatory in the eventual
production hot path.

### Still required before consensus activation

This stage does not propose consensus activation.

Remaining work includes at least:

- crash-consistent persistent authenticated-state generations,
- restart and power-loss fault injection,
- state-generation pointer recovery,
- randomized long-run reorg/property testing,
- large-state performance and memory benchmarks,
- independent security review,
- explicit same-chain activation and migration specification.

Until those requirements are satisfied, protocol-v8 state remains canonical.
