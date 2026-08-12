# NIGHTFALLCOIN — 100% Fair Launch

**Status:** Binding design intent for genesis.  
**Coin:** NIGHTFALLCOIN  
**Ticker (proposed):** `NIGHT`  
**Chain:** Nightfall L1 (sovereign, not a token on a foreign chain)

---

## 1. Definition of “Fair”

A fair launch means **no privileged claim** on the initial money supply.

| Principle | Implementation |
|-----------|----------------|
| No premine | Genesis creates **zero** spendable founder balance |
| No team bag | No `team`, `foundation`, `treasury`, `advisor` allocations at genesis |
| No private sale | No SAFT, seed, strategic, or OTC primary issuance |
| No insider emission | Emission schedule is identical for all participants |
| Public rules | Parameters published **before** genesis; hash of genesis config committed |
| Same software | No special binaries for insiders |

If it is not available to a stranger with a laptop on day one, it does not ship.

---

## 2. What Genesis Contains

Genesis **may** include:

- Consensus parameters (block time targets, epoch length, supply schedule)
- Protocol constants (max supply or asymptotic emission, if any)
- Empty or protocol-only system accounts that **cannot** be spent by humans
- Chain ID, network magic, protocol version

Genesis **must not** include:

- Allocations to named people, multisigs controlled by founders, or “ecosystem” wallets
- Hidden inflation switches
- Upgrade keys that can mint

---

## 3. How Coins Enter Circulation

**Only through open protocol rules**, for example (final mechanism locked in SPEC):

1. **Block rewards** to whoever produces/validates blocks under public consensus rules  
2. **Later (optional):** fees burned or redirected by transparent protocol logic  
3. **Never:** manual mint transactions, council mints, or “emergency issuance”

### Provisional emission philosophy (cypherpunk-hard)

| Parameter | Intent |
|-----------|--------|
| Issuance | Predictable, programmatic, front-loaded only via *public work/stake*, not via insider tables |
| Dev funding | **No forced dev tax at genesis.** If the community later wants a funding stream, it requires a hard-fork vote / social consensus — not a silent treasury |
| Max supply | **LOCKED: 90,000,000 NIGHT hard cap** — **no tail emission** |
| Fairness test | A random node joining at block 1 and a random node joining at block 100_000 play by the same rules |

**Emission (locked model D):** initial **20 NIGHT**/block, halving every **2_250_000** blocks, clamp to cap. Ideal geometric sum = 90M. Details: `docs/SPEC.md`, `docs/ATTRIBUTES.md`.

---

## 4. Founder / Contributor Rules

Contributors (including original authors):

- Receive **nothing** at genesis by special right
- May earn NIGHT **only** the same way as anyone else: run infrastructure, contribute work the market pays for, or buy on the open market after emission exists
- May accept **voluntary** community donations **after** launch — never protocol-enforced privilege
- Must not hold “protocol admin” keys

**Reputation is earned in the repo and the network — not in the allocation table.**

---

## 5. Anti-Patterns We Explicitly Reject

| Anti-pattern | Why it dies here |
|--------------|------------------|
| “5% team for 4 years vesting” | Preferential money supply access |
| “10% ecosystem / grants” | Soft premine with better PR |
| “Liquidity bootstrap wallet” | Insider market power at TGE |
| “Fair launch” + private seed | Lie |
| Retroactive founder mint | Exit scam energy |
| Foundation with mint authority | Bank with extra steps |

---

## 6. Transparency Checklist (pre-mainnet)

- [ ] Genesis config published + SHA-256 (or stronger) hash pinned in multiple places
- [ ] Full node + wallet source public (Rust)
- [ ] Emission schedule auditable in code (single source of truth)
- [ ] No privileged addresses in genesis alloc
- [ ] Network launch time announced in advance (no stealth insider farm if PoW; no stake-weighted insider headstart without open bonding window)
- [ ] Reproducible builds documented

---

## 7. Social Contract

> The protocol does not owe founders a fortune.  
> Founders owe the protocol nothing but correct code and no betrayal.  
> The market may reward builders — the genesis block must not.

**NIGHTFALLCOIN:** same night for everyone.

---

## 8. Ticker & Branding Note

| Item | Value |
|------|--------|
| Full name | NIGHTFALLCOIN |
| Chain name | Nightfall |
| Ticker | **NIGHT** (primary proposal) |
| Alt tickers | `NFC` (avoid confusion with NFC payments), `NFALL` |

Display forms:

- `NIGHTFALLCOIN`
- `Nightfall Coin` (prose)
- `$NIGHT` (markets / social)

**Disambiguation:** EY’s historical “Nightfall” Ethereum privacy research/tooling is unrelated. This project is a **sovereign L1** and native coin under community fair-launch rules — not an enterprise rollup product.
