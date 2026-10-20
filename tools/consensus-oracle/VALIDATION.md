# Independent Consensus Oracle — Validation Record

Base commit: `41357431ba8771d38d7d1f6d8bc77caf131fbdc4`

Full live validation performed on 2026-10-01.

## Results

- Phase 1: PASS
  - observed tip height: 265,718
  - 265,719 headers processed
  - cumulative work: 62,011,324,634
  - minted darks: 159,431,400,000,000
  - checkpoint 25,000 verified
  - emission, LWMA, MTP and parent linkage matched

- Phase 2A: PASS — 8 full headers independently hashed
- Phase 2B: PASS — 24 Nighthash-v2 samples across genesis/checkpoint/tip
- Phase 2C-1: PASS — 24 sampled block bodies
- Phase 2C-2: PASS — 8 real transfer blocks, 15 inputs, 24 outputs, 16 kernels
- Phase 2C-3: PASS — 2,327 blocks replayed, 15 ownership signatures, 2,327 UTXO-root matches, 2,327 kernel-sum matches
- Phase 2D: PASS — 180,384 deterministic adversarial assertions

The corrupted input-signature negative control was correctly rejected.

Nightfall production crates remained unchanged.

The subsequent compiler-warning cleanup only removes an unnecessary initial assignment and does not change the validation rule set.

These results document the stated test scope. They are not formal verification or an external professional security audit.
