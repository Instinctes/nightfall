# Internal swap review — 13 September 2026

> **WITHDRAWN — 18 September 2026.** Atomic swap is not part of NIGHTFALLCOIN.
> It is not in wallet 1.0.0 and is not scheduled for a later release. The code
> has been removed from the tree. This document is kept as a historical record
> only; it describes software that no longer exists in the product. See
> [`SWAP-WITHDRAWN.md`](SWAP-WITHDRAWN.md) for the decision and its reasoning.

In progress. This is an internal source/test review, not an independent audit,
security certification or Mainnet release approval. No real funds were used.

## C-SWAP-01 — Critical: cross-curve equality was not binding

Location: `crates/nightfall-crypto/src/dleq.rs`, former `system()` and
`generate_statement()` generator setup.

The former implementation used each curve's standard basepoint both for the
bit term and the blinding term of its Pedersen commitments. A malicious prover
who knows two **different** secret scalars could adjust the published blinding
sums and still pass the verifier. Honest prover/verification tests did not test
this construction: they always supplied the same scalar on both curves.

### Reproduction observed before the fix

`dleq::hardening_tests::different_secrets_cannot_be_hidden_in_the_published_blinding_sums`
failed with:

```
SECURITY: a proof linking secp secret 7 to NIGHT secret 11 was accepted
```

The test builds its own malicious witness and proof rather than modifying
bytes of an honest proof. Its commitments claim zero bits. With H=G, choosing
the public blinding sum as the sum of the commitment blindings minus the
desired secret independently on each curve makes the final unblinded points
match both independently chosen claims. All component proofs can then verify
without equality of the two secrets.

This invalidates the property required for the Bitcoin-revealed scalar to open
the corresponding NIGHT share. It can break counterparty claims/refunds;
successful honest swaps and the preexisting byte-tampering tests do not rule
it out. No public-network exploitation or real-wallet compromise is asserted.

### Correction implemented

- Distinct additional generators are derived directly as curve **points** from
  fixed versioned domain strings. Hashing to a scalar and multiplying G would
  expose the discrete-log relation and would not fix the issue.
- The final same-curve DLEQs use those additional generators, matching the
  structure of the upstream cross-curve construction. The upstream description
  uses commitments `r·G + b·H` and ties the unblinded `x·H` to the claimed `x·G`.
  See [sigma_fun 0.9.0 cross-curve source](https://docs.rs/sigma_fun/0.9.0/src/sigma_fun/ext/dl_secp256k1_ed25519_eq.rs.html)
  and its [Pedersen commitment explanation](https://docs.rs/sigma_fun/0.9.0/sigma_fun/).
- The Ristretto transcript leaf is explicitly domain-separated for v2.
- Proof format, packet format and saved session format are versioned. Old,
  unversioned and unsupported proof formats are not accepted; old sessions
  cannot automatically resume using the unsafe construction. Original files
  are preserved for a future explicit recovery procedure, not migrated by
  relabeling their proofs.
- The attack remains a regression test with current proof version 2, so its
  rejection tests the corrected cryptography, not merely an old-version gate.

Public generator vectors (not wallet keys):

```
secp256k1: 0390793fcf2f9697d636ccb8aa8a4dc78a62befce645394588e9330d5ae04f076b
Ristretto: acf4cdcf23bb13fd47a427d58b5cd3f689f692b4e8effc068a48059d53dcc70e
```

The standard NIGHT commitment generators, transaction consensus rules, wallet
seeds, emission and genesis were not changed. This is a swap-proof protocol
correction; it does not unlock the unrelated permanently locked NIGHT case.

### Verification checkpoint

After the generator correction, all 21 DLEQ tests passed, including the attack,
honest proofs, bit decomposition, leaf equations, minimum/maximum allowed
secrets and old/future proof rejection. Generator vectors are pinned in a test.

14 September post-fix rerun (legacy path, packet/session v2): 482 regular tests
passed — Crypto 103, Wallet 140, Swap 160, Core 79. Isolated Bitcoin-regtest /
NIGHT-devnet success and cancel/refund-with-NIGHT-recovery also passed, with
session reload before each step. That is evidence for the corrected legacy
executor, not for Vault execution, signed fee variants, or swap-backup recovery.
A full workspace/platform check was not part of that run. The cited `/private/tmp`
logs are no longer present.

### Remaining scope

Full internal review of every protocol transition, fee replacement graph,
Vault executor and backup recovery is still open. Legacy swap recovery needs a
separate deliberate path; do not recommend deleting old records or starting
over with locked funds. Mainnet and Vault execution gates remain closed.
Before any deployment the operator now requires a new local ARM dev wallet,
their own testing, and a final Nightfall-colored glassmorphism design pass.
