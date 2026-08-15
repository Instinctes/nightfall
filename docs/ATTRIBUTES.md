# NIGHTFALLCOIN — Locked Attributes

> ## ⚠ Reconciliation required — 2026-08-12
>
> The v4 implementation did not deliver several attributes locked below. The
> [security audit](./AUDIT-2026-08-12.md) fixed most of them in protocol v5, but
> **two claims in this document are still not backed by code**:
>
> | Locked claim | Reality after v5 |
> |---|---|
> | *Anonymity-set goal: **Protocol-scale / full-set class*** | **Partly delivered.** Blocks now aggregate every transaction into one sorted set, so the anonymity set is every transaction in the block rather than one. Cut-through is still absent and is incompatible with one-sided payments, so spent outputs remain visible and the graph is obscured rather than erased. |
> | *Network-layer privacy: **Dandelion-class or better*** | **Stem/fluff implemented** over the existing `Tx` message. Optional SOCKS5/Tor for outbound dials. A hop onto a node that still fluffs immediately remains a leak. |
> | *Bootstrap: **memory-hard / CPU-fair (not SHA-256 ASIC day-one)*** | ✅ **Delivered.** Nighthash-v2 is Argon2id at 32 MiB per hash on mainnet. |
> | *Max supply **90,000,000*** | Ceiling. v8 curve terminates at **89,999,999.25** (0.75 NIGHT short). |
>
> Delivered in v5: mandatory confidential amounts, no on-chain recipient,
> unlinkable one-time output keys, real user-controlled view keys, 0 premine
> enforced in code, 100 % fee burn, no admin keys, and a globally verifiable
> supply invariant.
>
> **Either amend these rows or build the features. Do not publish the gap.**


**Status:** **LOCKED** (community decision 2026-08-12)  
**Lock variant:** **D** — full research lock + **hard cap, no tail**  
**Max supply:** **90,000,000 NIGHT** (absolute ceiling, no tail emission)

This document is binding product intent. Protocol numbers that implement it live in [`SPEC.md`](./SPEC.md) and in code (`nightfall-types`, `nightfall-consensus`).

---

## Identity

| Attribute | Locked value |
|-----------|----------------|
| Name | **NIGHTFALLCOIN** |
| Chain | **Nightfall L1** (sovereign) |
| Ticker | **`NIGHT`** |
| Base unit | **`dark`** — 1 NIGHT = 10⁸ darks |
| Stack | **Rust** |
| Tone | Cypherpunk-hard |
| Product class | **Private settlement network** — not a Bitcoin remake, not an EVM clone |

---

## LOCKED — Economics

| Attribute | Value | Notes |
|-----------|--------|--------|
| Premine | **0** | Fair launch |
| Team / foundation / VC allocation | **0** | No privileged bags |
| **Max supply** | **90,000,000 NIGHT** | Ceiling. Curve terminates at 89,999,999.25. |
| **Tail emission** | **None** | After last subsidy the reward is 0. Fees then pay the miner. |
| Emission shape | 6 NIGHT, halving every 7,500,000 blocks | 50 % in ~3.6 y, 89 M in ~23.5 y |
| Fee model | Burned while a subsidy exists. After that, fees go to the miner (not minted, not burned). | |
| Dev tax in protocol | **0** at genesis | Voluntary community funding only |

**Why not 21M:** Explicit anti-clone choice. **90M** is own scarcity identity.  
**Why no tail:** User lock D — long-term security relies on **fees + voluntary infrastructure**, not perpetual inflation.

---

## LOCKED — Privacy

| Attribute | Value |
|-----------|--------|
| Default | **Mandatory shielded** value transfer |
| Sender / receiver / amount | Amounts ✅ · receiver ✅ (one-time keys) · graph **obscured by aggregation, not erased** ⚠ |
| Transparent path | Only explicit, rare unshield if any — never default |
| View keys | **Yes** (user-controlled) — ✅ implemented in v5 as `nfview1…` |
| ZK disclosure packs | **Yes** (prove without full open) |
| Admin freeze / blacklist keys | **Forbidden** |
| Anonymity-set goal | Block-level aggregation. **Not** protocol-scale. Do not advertise it as such. |
| Wallet UX | Anonymity-set / privacy strength as visible metric |
| Network-layer privacy | Dandelion-class stem/fluff ✅ · Tor/SOCKS5 optional ✅ · first hop to a pre-stem node still fluffs |
| Trusted setup | Prefer none; if ever used, public multi-party only |

---

## LOCKED — Consensus & launch

| Attribute | Value |
|-----------|--------|
| Launch ethics | **100% fair launch** |
| Bootstrap | **PoW**, memory-hard / CPU-fair — ✅ Argon2id (Nighthash-v2), 32 MiB/hash |
| Long-term | **Hardcoded path** to sustainable security (hybrid/PoS *allowed later* only if non-capturable and public); bootstrap remains fair PoW |
| Block time target | **~15 seconds** |
| MEV posture | Encrypted / blinded mempool path — extraction is a bug, not a business |
| Governance | Rough consensus + rare hard forks; **no mint authority** |

---

## LOCKED — Product scope

### v1 (must ship)

- Shielded **NIGHT** payments  
- Encrypted memos  
- View keys  
- Light / mobile-oriented proving path (design constraint)  
- **Private multi-asset note architecture** (stablecoin-capable by design; issuance policy later)  
- Swap / P2P-friendly design (delisting-resilient culture)  
- Fair genesis seal (0 allocations) enforced in code  

### v1.1

- ZK disclosure packs  
- Agent-ready account types (capability-limited)  

### v2

- Limited **private shared state** (e.g. private orderflow primitives) — not full public EVM  

### Explicit non-goals (v1)

- Full EVM clone  
- NFT / restaking / RWA-PDF theater as identity  
- Enterprise kill switches  
- Premine “ecosystem” treasuries  
- Optional privacy as default mode  
- Bitcoin narrative clone (21M religion, transparent gold-only)

---

## Positioning (one line)

> **NIGHTFALLCOIN is unreadable settlement money with a hard 90M cap, fair issuance, user-owned disclosure, and rails for private value — including stable-value architecture — without VC original sin.**

---

## Change control

| Change type | Rule |
|-------------|------|
| Soft (docs clarity) | Allowed anytime |
| Hard (supply, premine, privacy default, admin keys) | Requires explicit new lock + hard-fork social consensus |
| Pre-mainnet emission curve tweak | Allowed if **still sums to ≤ 90M** and **no premine** |

**Locked supply invariant:**  
`total_minted ≤ 90_000_000 NIGHT` forever. No tail. No emergency mint.
