# External review package

Prepared 30 September 2026 (Roadmap 2027, milestone 7). Local, unpublished.

**Status: no external review has taken place.** Every earlier review in
`docs/AUDIT-*.md` was internal (and the 8 September one AI-assisted). This
package exists so that an independent reviewer can start without a guided
tour: what to look at first, what an attacker wants, how to reproduce every
claim, and how to report. Nobody has been contacted or paid; choosing and
engaging a reviewer is the maintainer's decision.

## 1. What is being protected

| Asset | Where it lives | Worst outcome |
|---|---|---|
| Supply integrity (90 M cap, no inflation) | consensus + ledger | coins created from nothing, undetected |
| Spend authority | wallet keys (desktop vault, browser vault, phone) | theft |
| Privacy (amounts, recipients, links) | crypto (stealth outputs, commitments), wallet scan, light API | deanonymisation of payers/payees |
| Chain agreement | consensus fork choice, node runtime, storage | permanent split, lost confirmed payments |
| Availability | node P2P / RPC / light API | network or wallets unusable |

## 2. Scope, in priority order

Line counts from `wc -l` on 30 September 2026 (source only, tests included
where they live in the same file).

| Priority | Area | Entry files | Lines | Why first |
|---|---|---|---|---|
| **P1** | Supply & validity | `crates/nightfall-ledger/src/{utxo,lib,aggregate,tx,builder}.rs`, `crates/nightfall-consensus/src/{lib,difficulty}.rs` | ≈3 500 | inflation or invalid-block acceptance is unrecoverable |
| **P1** | Cryptography | `crates/nightfall-crypto/src/{kernel,rangeproof,stealth,keys,pow,mnemonic}.rs` | ≈2 500 | kernel signature over H, Bulletproof use, one-time keys, view tags, seed derivation |
| **P2** | Fork choice & persistence | `crates/nightfall-node/src/runtime.rs` (reorg, catch-up, mining wait), `crates/nightfall-storage/src/{lib,codec,dirlock}.rs` | ≈5 000 | a confirmed payment vanishing; two writers on one chain file |
| **P2** | Network surfaces | `crates/nightfall-p2p/src/lib.rs`, `crates/nightfall-node/src/{session,rpc,mobile}.rs` | ≈1 700 | DoS, amplification, RPC exposure; the light API allow-list and rate limit (X-Forwarded-For trusted from loopback only, added 30 Sep) |
| **P2** | Key storage | `crates/nightfall-wallet/src/{vault,vault_store,vault_fs,recovery}.rs`, `crates/nightfall-web/src/vault.rs`, `website/public/wallet-origin/{vault-store,session,app}.js` | ≈4 500 | vault format, migration, browser storage, two-phase send |
| **P3** | Payment semantics | `crates/nightfall-wallet/src/{payment_request,counter,receipt}.rs`, `website/public/wallet-origin/pay-link.js` | ≈1 900 | a request is not a payment; till settlement and confirmation depth (added 30 Sep) |
| **P3** | Website Worker | `website/src/index.js` (wallet proxy, `anchoredWalletScan`, CSP, canonical host) | ≈700 relevant | light-wallet trust boundary, origin separation |
| **P3** | Operator tooling | `crates/nightfall-nodekit` | ≈3 200 | must never write outside the path named, never publish RPC |

**Out of scope:** atomic swaps (withdrawn in 1.0.0; `docs/SWAP-WITHDRAWN.md`),
the desktop UI layout code in `nightfall-core/src/views.rs` except where it
handles keys or sends, build/deploy scripts, the marketing site.

## 3. Threat model (summary)

The full internal analysis is `docs/AUDIT-2026-08-16.md` (§2 onward) and
`docs/AUDIT-2026-09-08.md`. The attacker types a reviewer should hold in mind:

1. **Block producer / miner with some hashrate** — invalid blocks, deep
   reorgs, timestamp games against LWMA-1, selfish mining. *Known open:* the
   network's hashrate is small (AUDIT-2026-08-16 H-01); a reorg of a few
   blocks is affordable. Checkout therefore reports confirmation depth.
2. **Malicious peer** — malformed or oversized messages, header floods, fake
   heights to stall mining (`mining_should_wait`), eclipse via address gossip.
3. **Malicious or compromised light-API node** (seed or self-hosted) — can
   hide or fabricate *consistent* chain data to a light wallet, sees IPs and
   submitted transactions; cannot spend. By design (L-01); the browser
   checks genesis, page continuity and anchor hashes, not proof of work.
   `docs/MY-NODE.md` states this to users.
4. **Local attacker on the user's machine** — seed/vault files, permissions,
   plaintext fallbacks (refused by design), RPC on loopback.
5. **Web attacker** — XSS or injected script on the wallet origin, cross-
   origin reads, a phishing clone. Mitigations: separate wallet origin,
   `connect-src 'self'`, no inline script, `frame-ancestors 'none'`.
6. **Payer/payee dispute** — "I paid" vs "nothing arrived": receipts
   (`verify-receipt`), memo-reference matching, candidates never applied.

Properties the project *claims* and a reviewer should try to break:

- no transaction or block can increase supply beyond the emission schedule;
  `verify_supply` recomputes it from the UTXO set;
- a range proof exists for every output and binds to its network context;
- the kernel signature covers the transaction body;
- an observer of the chain cannot link an output to an `nf1` address;
- a pruned node refuses to serve the light API;
- the browser refuses any scan page that does not continue its saved anchor;
- no RPC method that mines or reveals wallet data is reachable off loopback.

## 4. Reproduce everything

Toolchain: Rust **1.98.0** (CI pin), Node ≥ 20. All offline once
dependencies are vendored/fetched; nothing contacts mainnet.

```
scripts/review/reproduce.sh            # writes output/review/<timestamp>/summary.md
```

It runs, and records the exact result of each:

| Step | Command |
|---|---|
| format | `cargo +1.98.0 fmt --all -- --check` |
| lints | `cargo +1.98.0 clippy --workspace --all-targets --offline -- -D warnings` |
| workspace tests | `cargo +1.98.0 test --workspace --offline` |
| exploit regressions | `cargo +1.98.0 test -p nightfall-ledger --test exploit_regression --offline` |
| emission | `node scripts/check-emission.mjs` |
| browser wallet (real WASM) | `node scripts/check-web-wallet-v104.mjs`, `check-pay-link-wasm.mjs`, `check-backup-rehearsal.mjs` |
| wallet scan loop / My Node | `node scripts/check-wallet-sync.mjs`, `check-wallet-my-node.mjs`, `check-wallet-scan-proxy.mjs` |
| website security | `node scripts/check-website-security.mjs` |
| multi-node devnet | `node scripts/lab/run-lab.mjs`, `node scripts/lab/checkout-devnet.mjs` |

Genesis to expect on mainnet (computed from consensus by
`nightfall-nodekit genesis`):
`061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de`.

## 5. Reporting

Use `docs/review/FINDINGS-TEMPLATE.md`, one file per finding. Severity by
impact on the assets in §1:

| Severity | Meaning |
|---|---|
| Critical | supply inflation, theft of funds, or key disclosure without user action |
| High | loss of a confirmed payment beyond the documented reorg risk, deanonymisation at scale, remote crash of nodes |
| Medium | DoS requiring sustained resources, privacy leak needing a privileged observer, unsafe default |
| Low | defence-in-depth, misleading UI text, documentation contradicting code |

Findings go to the maintainer privately first. Nothing in this package
promises a bounty, a timeline or publication terms; those are for the
maintainer to set with the reviewer.

## 6. What a reviewer will not find here

A claim that the software is secure. A prior external audit. Independent
reproduction of release builds (`docs/RELEASE-VERIFY.md` explains how to
compare, and records that nobody outside the project has done it yet).
