# Swap implementation checkpoint — 13 September 2026

> **WITHDRAWN — 18 September 2026.** Atomic swap is not part of NIGHTFALLCOIN.
> It is not in wallet 1.0.0 and is not scheduled for a later release. The code
> has been removed from the tree. This document is kept as a historical record
> only; it describes software that no longer exists in the product. See
> [`SWAP-WITHDRAWN.md`](SWAP-WITHDRAWN.md) for the decision and its reasoning.

Status: storage foundation and legacy-path corrections, **not a completed
Vault/Mainnet swap release**. No deployment, release upload or Git push in
this work. The previously delivered ARM dev.2 archive predates these edits.

## Updated release policy

The operator explicitly requests complete internal verification and no external
review. This supersedes earlier planning that made a third-party review a
release dependency. It does not turn tests into an independent audit or a
security guarantee. The Mainnet gate remains closed for concrete unfinished
implementation and verification work, not because an external reviewer is
scheduled. Do not claim that the custom cross-curve proof was independently
reviewed.

## Implemented boundary

- `nightfall-wallet::swap_journal`: versioned bounded recovery records inside
  the wallet snapshot; empty snapshots retain the old shape. Revisions use
  compare-and-swap so a stale worker cannot overwrite a later commit. Legacy
  plaintext writers refuse nonempty recovery journals. Unsupported versions
  fail closed, including empty journals.
- Session checkpoint encoding/decoding is separate from legacy `.secret`
  files. Decode binds the expected swap ID/network and attaches no filesystem
  writer. Serialized secret buffers use zeroizing owners. This is not a claim
  that every transient copy or every Session secret is erased from RAM.
- Core has a combined versioned handshake/execution envelope. It rejects
  mismatched IDs, roles, amounts, networks and invalid deadlines; edits cannot
  change agreed terms. A fresh setup cannot import an advanced/legacy session
  and silently reset its execution state. Both creation and editing retain
  the Mainnet gate.
- `VaultStore::update` commits the combined checkpoint and wallet candidate
  together before returning a packet or intent. Failed transactions do not
  expose candidate results. Post-rename uncertainty requires reopening.
- Rescan and chain-maintenance guards preserve journals. Normal keys-only and
  full-state backup restoration refuse swap-bearing backups until dedicated
  chain reconciliation exists. Backup export and verification retain records;
  original backup files are not modified. There is intentionally no automatic
  retirement/deletion API for recovery material yet.

The Core adapter **is connected** on 16 September for unlocked non-mainnet
Vault wallets. Handshake, execution, reservations and outbound intents share
one `VaultStore::update`. The legacy file executor remains for plaintext
wallets. Mainnet stays closed. Dedicated backup recovery, signed fee variants
and journal retirement are still absent.

## Legacy executor corrections

- Direct App and Worker entry points enforce actual custody/network, including
  cached sessions. App also checks its paused state. UI labels are not custody
  authority. These checks are not a race-safe execution lease for Vault.
- Manual actions validate the saved execution record against the handshake
  before locking or broadcasting, not only in the subsequent watch loop.
- Failed reservation release preserves recovery files and in-memory
  reservations. Reserve/release failures roll back their in-memory candidate.
- Clearing wallet views drops cached handshake ownership. Mainnet records are
  explicitly read-only; this build does not secretly operate recovery while
  saying that swaps are disabled.
- Fee proposals reject invalid/oversized/unordered rungs and requests above
  the ceiling. Their explicit budget is per transaction, not the whole abort
  graph. The unchanged 2,000-sat blank draft default belongs to the existing
  **single-fee protocol**, not to a signed ladder. Selecting a helper rung
  does not create an eligible replacement transaction.

## Verification

The first focused storage run passed seven tests. The first broad Core run
passed 76 tests and failed two new checkpoint tests: pretty-printed proof arrays
exceeded the proposed 2 MiB budget. Checkpoints now use compact JSON; legacy
pretty-printed inputs have a separate 16 MiB cap, enforced before allocation
and while reading. The complete two-Vault M0–M5 handshake is tested against the
combined budget, not just an empty session. Full updated wallet, swap and Core
suites were rerun. At this storage checkpoint: Core 79 passed (2 normally
ignored), wallet 140 passed (1 subprocess helper normally ignored), and swap
159 passed across unit/integration targets (8 live/long-soak tests ignored).
A stale UI assertion requiring an outside reviewer was updated to assert the
actual unfinished technical gates and internal-review policy; no gate was opened.
The isolated Core Bitcoin-Regtest/NIGHT-Devnet success/refund lifecycle passed
separately (177.80 s), after a sandbox denied the initial loopback bind. The
WASM webwallet target check also passed. The counts above **precede the
generator-binding correction** and are not post-v2 evidence.

14 September post-fix counter-check: 482 regular tests passed — Crypto 103,
Wallet 140, Swap 160, Core 79 — plus the isolated Bitcoin-regtest / NIGHT-devnet
success and cancel/refund-with-NIGHT-recovery path, with session reload before
each step. That confirms the legacy executor after proof/packet/session v2. It
does not connect the Vault adapter, lock ownership, dedicated recovery or a
signed fee ladder. No gate was opened. The `/private/tmp` logs from that run
are no longer present. `SWAP-SPEC-DRAFT.md` §5 still claims the generator is
not a problem and must be corrected before release documentation is frozen.
Scoped Clippy for all targets of the three crates passed with warnings denied.
Two preexisting test-only lint issues in receipt/counter fixtures were corrected
without changing their assertions or production behavior.
Fault injection covers partial writes, synchronization, rename, no outbound
result on failed persistence, lock/restart/password rotation and backup
preservation. Session tests restart between M0–M5 and verify claim-secret
recovery. These are internal tests, not power-cut hardware tests or an audit.

## Remaining implementation sequence

1. **Vault executor and lock ownership.** Done 16 September for unlocked
   non-mainnet Core: worker/UI persist through the encrypted journal; locking
   is refused while a swap is unfinished or a job is in flight; cached secrets
   are dropped on lock; both chains must refresh after unlock before a spend;
   admission reserves later journal capacity and funding re-checks it. Not a
   live Bitcoin/NIGHT vault-swap run and not a Mainnet opening.
2. **Dedicated swap restoration.** Reconcile a checkpoint/backup against both
   chains, detect stale intents and retain known transaction variants. Never
   replay an old backup as a new funding transaction. Define terminal
   reconciliation and safe journal retirement before enabling creation.
   Unsupported hosts (including a browser without a swap executor) must not
   activate swap-bearing backups as ordinary restored wallets.
3. **Versioned signed-fee protocol.** Negotiate and authenticate all variants;
   sign the correct cancel/refund/punish parent-child graph; enforce cumulative
   fee/output budgets. Extend driver persistence/observation to competing
   variants, replacements, confirmation and reorgs. Packet v1 has only one
   fee and one signature per leg; incompatible variants must not masquerade as
   old packets.
4. **Internal security review and lifecycle tests.** Review the custom proof,
   transcripts, transaction graph and amount arithmetic. Exercise both roles,
   success/cancel/refund/punish, restart at every boundary, failed RPC/storage,
   fee pressure, reorgs, stale backups and lock/unlock. Short regtest deadlines
   are not evidence of weeks of unattended public-network operation.
5. **Permanent NIGHT-lock policy.** The current construction can compensate
   Alice with Bitcoin after Bob fails to refund, but cannot unlock her NIGHT.
   See `SWAP-LOSS.md` and `SWAP-SPEC-DRAFT.md`; this is not resolved by a Vault
   checkpoint or more tests. A claim of unconditional NIGHT refund needs a
   different, validated protocol and potentially consensus changes, not a UI
   promise. No consensus/genesis/emission change has been made here.
6. **Release and deployment only after the implemented scope passes.** Build
   fresh artifacts, verify version/network isolation and downloads, then use
   the established deployment process within the operator's Git/Cloudflare
   permissions. Do not re-label an older dev archive as the completed release.
