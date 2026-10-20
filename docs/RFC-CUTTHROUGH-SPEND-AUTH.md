# RFC: Cut-through-compatible one-sided spend authorization

Status: experimental design / non-consensus
Target: protocol research for a future explicitly versioned upgrade

## 1. Problem

Nightfall currently combines:

- non-interactive one-sided stealth outputs
- per-output one-time spend keys `Ko`
- per-input Schnorr authorization under the referenced output's `Ko`
- block-level transaction aggregation

This prevents a sender from reclaiming an output after paying another wallet.

It also prevents Mimblewimble cut-through.

Today an input remains in the block because its Schnorr signature is the
receiver-ownership proof. Removing an output/input pair created and spent
inside the same block would also remove that authorization evidence.

The objective is therefore not merely to delete matching commitments.

The objective is:

> preserve receiver-only spend authorization after an intermediate
> output/input pair is removed from retained chain history.

## 2. Non-negotiable invariants

Any acceptable design MUST preserve all of the following.

### 2.1 One-sided payments

The receiver does not need to be online when the payment is created.

### 2.2 Sender cannot reclaim

Knowing the output commitment blinding factor and sender-side ephemeral
secrets must not give the sender authority to spend the receiver's output.

### 2.3 Supply soundness

The existing Nightfall value invariant remains authoritative:

    Σ UTXO - Σ kernel_excess = (minted - burned) * G

Cut-through must not weaken, replace, bypass, or approximate this invariant.

### 2.4 UTXO authority

Spend authorization must be checked against the canonical UTXO being spent.

Input-supplied metadata must never be trusted as a substitute for canonical
UTXO metadata.

In particular, the one-time output key `Ko` remains state-derived.

### 2.5 Aggregation

Independent transactions must remain safely aggregatable without preserving
transaction boundaries.

### 2.6 Atomic failure

A failed authorization, balance check, metadata check or cut-through check
must leave canonical ledger state unchanged.

## 3. Reference construction

The primary reference is Litecoin LIP-0004, One-Sided Transactions in
Mimblewimble.

LIP-0004 separates receiver spend authorization from the ordinary MW value
balance by introducing an additional stealth-key balance.

Conceptually:

    output sender keys
      +
    input ephemeral keys
      -
    spent output keys
      =
    stealth excesses
      +
    stealth offset * G

The construction additionally uses an input proof tied to both:

- a fresh input ephemeral key `Ki`
- the canonical output key `Ko`

and binds the stealth excess into kernel authorization.

This RFC does not assume the Litecoin representation can be copied unchanged.
Nightfall uses Ristretto, its own domain separation, its own kernel balance
orientation and its existing v8 consensus objects.

The algebra and security properties must therefore be reproduced and tested
in Nightfall's model rather than translated mechanically.

## 4. Candidate mapping to Nightfall

Current Nightfall outputs already contain:

    commitment
    ephemeral_pk
    output_pk
    sender_sig

where:

    output_pk = Ko

and the sender proves knowledge of the secret corresponding to
`ephemeral_pk`.

A candidate design will investigate whether the existing ephemeral sender key
can safely provide the output-side key contribution required by the stealth
balance.

This is NOT assumed safe yet.

Before reuse is accepted we must rule out:

- cross-protocol key-reuse hazards
- rogue-key constructions
- signature rebinding
- aggregation cancellation attacks
- sender reclaim
- malicious metadata substitution

If reuse is unsafe, a distinct sender authorization key must be introduced.

## 5. Input model under investigation

A future experimental input may conceptually contain:

    commitment
    Ki
    authorization_signature

The canonical `Ko` MUST continue to come from the UTXO set.

The authorization proof must demonstrate knowledge associated with both
`Ki` and the secret spend key corresponding to canonical `Ko`.

The exact transcript and domain separation are part of the prototype and
must not become consensus-active until independently reviewed.

## 6. Stealth excess

The prototype will investigate an additive stealth authorization equation
compatible with aggregation.

Required properties:

1. valid independently created transactions aggregate
2. aggregation order does not matter
3. deleting an eligible internal input/output pair preserves the required
   global equations
4. an attacker without the receiver spend secret cannot construct a valid
   replacement authorization
5. the stealth authorization cannot be detached from the value kernel
6. arbitrary offsets cannot turn an invalid authorization into a valid one

## 7. Cut-through horizon

Immediate pruning is not assumed.

Authorization material may need to remain available for a consensus-defined
horizon before historical input/output pairs can be removed.

The horizon must be designed together with:

- Nightfall's maximum reorg depth
- pruning
- restart/replay
- authenticated state
- snapshot/bootstrap behavior

No horizon value is selected by this RFC.

## 8. Mandatory adversarial tests

No consensus proposal is acceptable until all of these are implemented.

### Ownership

- legitimate receiver can spend
- original sender cannot reclaim
- unrelated third party cannot spend
- substituted `Ko` fails
- substituted `Ki` fails
- substituted commitment fails

### Metadata integrity

- input metadata inconsistent with the canonical UTXO fails
- malformed group points fail
- duplicate authorization material fails
- canonical UTXO metadata always wins over peer-provided metadata

### Aggregation

- two valid transactions aggregate successfully
- aggregation order produces the same result
- stealth balances compose exactly
- value balances compose exactly
- kernel authorization remains bound to stealth authorization

### Cut-through

- eligible internal output/input pair can be removed
- removal preserves value balance
- removal preserves stealth authorization balance
- unauthorized deletion fails
- partially deleted authorization data fails
- cut-through cannot remove coinbase maturity enforcement

### Reorg / persistence

- reorg inside retained authorization horizon succeeds
- invalid competing branch fails atomically
- restart reconstructs exactly the same authorization state
- pruned restart fails closed when required state is missing

### Historical exploit regression

All existing `exploit_regression.rs` tests remain unchanged and passing.

## 9. Protocol boundary

The first implementation stage is deliberately NON-CONSENSUS.

It must not change:

- `PROTOCOL_VERSION`
- `WIRE_VERSION`
- block serialization
- block hashing
- genesis
- existing v8 validation
- emission
- supply accounting
- existing UTXO root semantics

A later consensus proposal, if justified, requires a separate review,
explicit protocol versioning and migration/reset analysis.

## 10. Acceptance criteria for the prototype

The prototype is successful only if it demonstrates all of the following:

- receiver-exclusive spend authority
- exact additive aggregation
- exact cut-through algebra
- no weakening of the existing value invariant
- canonical-state metadata binding
- deterministic adversarial regression coverage
- no dependency on transaction boundaries

Passing ordinary happy-path tests is insufficient.

## 11. Rejection criteria

The construction must be abandoned or redesigned if any of the following
cannot be demonstrated convincingly:

- sender-reclaim resistance
- canonical UTXO binding
- rogue-key resistance
- safe aggregation
- safe reorg handling
- safe pruning/restart semantics
- independent compatibility with Nightfall's supply invariant

Privacy improvement is not sufficient justification for weakening money
soundness.

## 12. Planned implementation sequence

1. freeze the security contract in this RFC
2. implement a non-consensus algebraic prototype
3. add receiver/sender/adversary tests
4. add aggregation tests
5. add cut-through transformation tests
6. add malicious metadata tests
7. add reorg/persistence model tests
8. run differential and property testing
9. document unresolved assumptions
10. only then evaluate whether a versioned consensus proposal is justified
