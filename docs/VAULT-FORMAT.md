# Nightfall Vault format 1 - development specification

Status: pre-release implementation for the Wallet 1.0.0 workstream. Not deployed.
Storage integration and migration remain release blockers. Do not manually
replace a production seed file with this format.

## Threat model

The envelope protects a copied encrypted snapshot against disclosure and
modification without its password, subject to password strength and the
cryptographic assumptions. The password is not a BIP39 passphrase and does not
change addresses. The 24 recovery words still encode the original wallet seed.

It does not protect an unlocked wallet against hostile first-party code,
extensions, a compromised device, screenshots, a keylogger or all memory copies.
It does not hide the network, protocol or approximate wallet-state size. It
does not detect replacement with a previous authentic snapshot or another valid
wallet encrypted under the same password. Rollback and wallet-identity binding
need a host-side policy; AEAD is not a monotonic counter.

Password rotation changes the current salt and encryption key. Old backups
remain decryptable under their old passwords. Deleting a legacy file does not
guarantee removal from SSD snapshots, cloud backups or forensic storage.

## Binary envelope

All multi-byte integers use little-endian encoding. The complete 55-byte header
is authenticated as AEAD associated data. No metadata is trusted merely because
the header parses successfully.

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 8 | ASCII `NFVAULT` followed by a zero byte |
| 8 | 1 | Format version: 1 |
| 9 | 1 | KDF/cipher profile: 1 |
| 10 | 1 | Network: 0 mainnet, 1 testnet, 2 devnet |
| 11 | 4 | Protocol version: currently 8 |
| 15 | 16 | Random KDF salt |
| 31 | 24 | Random XChaCha20-Poly1305 nonce |
| 55 | variable | Encrypted UTF-8 wallet export, followed by its 16-byte tag |

Profile 1 uses Argon2id version 0x13, m=65536 KiB, t=3, p=4 and a 32-byte
output as the XChaCha20-Poly1305 key. This follows the 64 MiB parameter option
in [RFC 9106](https://www.rfc-editor.org/rfc/rfc9106.html#section-4).
Implementation APIs: [RustCrypto Argon2](https://docs.rs/argon2/0.5.3/argon2/)
and [XChaCha20-Poly1305](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/).

Password input is exact UTF-8: no trimming, case conversion, normalization or
truncation. This version accepts 12 or more Unicode scalar values and at most
1024 UTF-8 bytes, rejecting all-whitespace passwords. This policy is not an
entropy estimate. The UI should encourage a long, unique password, never the
wallet recovery words. A future format must not silently change this encoding.

Salts and nonces use fallible OS/browser CSPRNG calls. A routine snapshot
reuses the unlocked derived key and salt, but samples a fresh 192-bit nonce.
Creating a vault or changing its password samples a fresh salt and key.

Reject more than 16 MiB of plaintext or 16 MiB + 71 bytes of envelope data.
Unknown formats, profiles, protocol versions and network values fail before
the KDF. There are no attacker-selected KDF costs or automatic downgrade paths.
Large wallet-state chunking needs a separate design before raising this bound.

## Plaintext schema and session ownership

The payload is wallet export version 1 with required `v`, `network`, `seed`
and `db` fields. Unknown top-level fields, duplicate fields, unsupported
versions, missing fields and unknown network names are rejected. Never default
an invalid network to mainnet or a missing scan database to an empty wallet.
After authentication, the payload network must match the authenticated header
and the caller's explicitly selected network.

`Vault` owns the encrypted snapshot and, while unlocked, the derived key and
an in-memory `Wallet`. It has no Debug, Clone or Serialize implementation.
Locked wallet access returns an error. Locking first creates a fresh encrypted
snapshot and only then drops the unlocked state; if snapshot creation fails,
the session remains available and the previous ciphertext remains intact.
The ciphertext remains available after lock so a failed storage write can be
retried without retaining the plaintext wallet.

Argon2 working memory and the derived key use zeroizing buffers. Transient
vault plaintext is zeroized on drop. Wallet drop clears key material and
sensitive scan/history fields. Mnemonic temporary strings and entropy are
cleared. These are best-effort application memory measures, not promises about
compiler temporaries, copies, OS swap, browser memory or cryptographic library
internals. UI/JS callers must clear their own secret fields and previews.

## Host integration requirements (not yet complete)

1. Serialize scan, sign, save, password change, migration and lock operations.
2. Persist an authenticated encrypted snapshot before broadcasting a payment.
3. Use atomic, owner-restricted, durable disk writes on Core; handle quota,
   denied storage, multiple tabs and interrupted writes in the browser.
4. Never mark a migration complete before independently reopening the saved
   ciphertext and comparing complete wallet identity/state. Retain the original
   on failure. Legacy retirement must be explicit and must not create a new
   hidden plaintext backup. Check for stale legacy temporary files as well.
5. On restart, an encrypted-vault marker forbids automatic plaintext fallback
   or generating a replacement seed. Unsupported files must be preserved.
6. Coordinate auto-lock with background jobs. Clear forms, displayed keys and
   pending preview copies. A merely hidden window or disabled Send button is
   not an encrypted lock.
7. Move the webwallet to an isolated origin and review its deployment and
   migration path. A separate path on the website is not a separate origin.
8. Run KDF/unlock work off the browser UI thread and benchmark real mobile
   devices. The native/WASM Node test is not Safari or Android validation.

## Native persistence adapter (development Core integration)

`nightfall_wallet::vault_store::VaultStore` is now implemented for Unix.
Tests run on macOS; Linux runtime verification and a Windows durability/ACL
implementation are still required. Windows enrollment returns an error before
creating any vault directory. This is not a production migration instruction.
The installed wallet and its real data have not been migrated.

The store retains an `Arc<DirLock>` for the original data directory and takes a
second writer lock inside `<seed-name>.vault/`. This prevents two store objects
from writing the same wallet even if their callers share the original guard.
Legacy writers must be quiescent before enrollment. Development Core pauses new
scans, waits for the wallet mutex and moves the wallet into its background vault
worker. No live legacy writer survives creation of the no-downgrade marker.

For `core.seed`, the intended layout is:

```text
data directory/
  core.seed                         non-secret, non-hex downgrade marker
  core.seed.vault/
    .nightfall-lock                  OS-managed writer lock
    wallet.nfv                       committed encrypted snapshot
    pending-<random>.nfv             possible interrupted encrypted writes
```

Creating the vault directory is explicit enrollment, not a read-only probe.
The directory itself forbids legacy open/create/save, even if a crash prevented
the first snapshot. It must not be deleted to bypass the guard. Interrupted
temporary snapshots are never promoted automatically. After a successful
retirement, the original seed contents are replaced with a non-secret marker:
pre-vault versions already refuse non-hex seeds and thus cannot silently create
a replacement wallet. Fresh vault provisioning writes the same marker.

Migration reads the original seed and database strictly, without calling the
legacy write-capable loader. Corrupt/unsupported JSON, missing required fields,
unknown top-level database fields, symlinks, hard links, non-regular paths,
oversized files and an outstanding `.outputs.json.tmp` cause an error. No bad
database is treated as an empty wallet. Older optional database fields retain
their established defaults. The initial encrypted snapshot is reread from disk,
unlocked independently and compared against the complete original wallet state.

Plaintext retirement requires a separate explicit password confirmation and
another verification of all surviving original files. Only then is the old
database removed and the seed atomically replaced with the marker. An interrupted
retirement can resume from its surviving files. If old and encrypted state
differ, neither original is retired. There is no new plaintext backup. Retirement
does not remove contacts or preferences and does not erase SSD snapshots,
previous backups or forensic copies.

`update` edits a private in-memory candidate. Its result, including a signed
transaction, is returned only after the encrypted commit succeeds. The closure
must not broadcast or perform other external side effects. Network and wallet
identity cannot be replaced through an edit. Password rotation follows the same
candidate/commit discipline. The public adapter exposes no mutable live wallet.
Lock therefore drops an already committed session without requiring another
write; it cannot lose an unpersisted successful edit.

Writes use a random, exclusively created 0600 temporary file in the same 0700
directory, sync the file, rename it over the snapshot, then sync the directory.
The adapter also refuses to overwrite a snapshot changed/deleted outside the
session. A failure after rename makes that session unusable until reopened;
it never reports a successful edit from an uncertain commit. These semantics
follow [Rust's rename API](https://doc.rust-lang.org/std/fs/fn.rename.html),
[File::sync_all](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)
and the separate directory-sync requirement described by
[fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html).

Limits: filesystem calls depend on OS/filesystem/hardware behavior; process-exit
tests are not power-loss tests. Locks are advisory, and paths are within a trusted
data directory, not a defense against a malicious process with the same OS user.
Rollback across restarts still requires a separate trust anchor. Interrupted
encrypted temporary files can retain earlier state/password protection; a future
cleanup/export UX must account for them. Old encrypted copies remain decryptable
with the old password. No guarantee of forensic erasure or independent audit.

Core treats damaged and orphaned storage as a load error: no node start, fake
zero-balance dashboard or first-run wizard. Existing valid vaults start locked;
an interrupted migration has its own resume screen, never a plaintext fallback.
Legacy Core storage is still used for unmigrated wallets. Development Core
create/restore onboarding now uses encrypted-first provisioning on Unix; it
does not call the legacy plaintext restore writer. Unsupported platforms refuse
new setup rather than quietly creating a plaintext wallet.

### Encrypted-first Core setup (10 September development)

New keys exist only in memory before provisioning. The user reveals and writes
down the 24 words, hides them and re-enters the complete phrase. Only full
key-derived address equality passes the backup check. Password confirmation,
password policy, checksum/length and an explicit recovery notice are checked
before a worker request is accepted. The backend repeats password/phrase and
existing-file checks before creating the permanent enrollment marker.

The worker creates an in-memory wallet at birth height zero, then calls
`VaultStore::initialize`: encrypted commit, reread/decrypt/compare and nonsecret
legacy tombstone. It never writes a plaintext seed or output database. Provisioning
returns locked; the UI discards setup secrets and requires a separate unlock
before starting the node. Restoration does not broadcast and scans from genesis
after unlock. Valid Bitcoin/Ethereum words do not recover those coins in Nightfall.

Focus loss, hiding and five minutes without input conceal generated words and
clear entered restore words, passwords and backup-verification input. An unsaved
generated identity is retained, concealed in zeroizing application memory until
explicit cancellation or successful provisioning; focus loss must not silently
generate another identity. Egui/OS copies are not guaranteed to be erased.

An empty enrollment marker after restart offers only explicit restoration from
the original words, never automatic generation. An encrypted snapshot without
its tombstone resumes password verification/retirement, not initialization. A
snapshot that exists, retired seed marker or any legacy database cannot be
replaced by setup. Damaged snapshots fail closed. Without an original committed
snapshot an original-word restoration cannot prove it matches lost files; retain
the original backups and do not treat a valid mnemonic checksum as that proof.

Nine Core tests cover full-backup mismatch, invalid credentials and phrases,
focus/idle concealment, six setup card states at 320/620/760 px, background worker
ownership, encrypted-only files, existing-data refusal, interrupted setup recovery
and application publication/restart without node startup. Interrupted disk states
in these Core tests are constructed fixtures; the separate adapter tests provide
actual injected process exits. Native GUI/manual and platform QA remain open.

### Core lifecycle and controls (10 September development)

Settings now offers a two-stage migration: password/confirmation and explicit
offline-backup notice to create the verified encrypted copy; then password
re-entry and an explicit action to retire legacy secrets. After retirement the
wallet remains locked until explicitly unlocked. Invalid passwords, mismatching
confirmation and a wrong directory or network refuse enrollment. The earlier
rule that an unfinished experimental swap also refused enrollment was removed
with the feature itself; see [SWAP-WITHDRAWN.md](SWAP-WITHDRAWN.md).

The application, store and scanner retain the data-directory lock. Password
derivation, migration, rotation and payment preparation execute in background
workers which temporarily own the wallet. New scans pause while they work.
Only the foreground poll can return an unlocked session to the shared wallet;
focus loss during an unlock/rotation causes the result to be locked first.

Automatic lock is requested after five minutes without input, on focus loss
or hiding the window. An in-flight operation may finish first. The UI must show
securing/in-progress until lock really completes, not claim the keys are gone
while a scanner or payment still owns them. Lock requests remain pending across
worker-start failures. Already running node/mining work can continue without
wallet access; wallet scanning and payment preparation stay paused while locked.
Revealed-key flags, recovery input, password fields and local form/preview buffers
are cleared on lock. This is best-effort memory hygiene, not erasure of egui/OS
copies, clipboard contents, backups or separately stored swap secrets.

Core's normal payment path now records the complete pending transaction before
its first node submission. With Vault, the encrypted candidate must be committed
before the transaction is returned to the submission path. Failure to submit
does not erase a durable pending record; users must inspect Activity before
creating a replacement payment. Scans, reservations and rescans also use the
transactional store. Pure local tests construct public Devnet outputs/signatures;
they never broadcast a real payment or start the user's node.

Remaining release checks include encrypted-backup recovery/import UX,
Windows and actual Linux execution, native GUI/manual review,
long-scan UI responsiveness, real isolated-node payment/scan lifecycle tests,
and full browser integration. Headless egui layout tests do not replace these.

### Encrypted backup export and rehearsal (10 September development)

An unlocked Core Vault can export its committed snapshot to an explicitly
entered absolute filename outside the active data directory. The chosen parent
directory must already exist; there is no tilde or environment expansion. Export
requires the current vault password again. The source is read/verified against
the in-memory wallet before creating any output. New files use exclusive creation
and Unix mode 0600: existing files, symlinks and hard links are never overwritten.
The ciphertext is written, file-synced, directory-synced, reread, authenticated and
compared with the entire current wallet state before success is reported. The
active wallet is not changed. Prefer a separate offline storage device.

This is deliberately not overwrite-based or atomic replacement of a prior backup.
A failed write/process exit can leave an incomplete NEW file; it is not reported
as verified, not promoted or silently deleted. Keep prior known-good copies and
retry with a different filename. Four injected export checkpoints test failure
reporting and preservation; unlike the adapter's commit-crash suite these new
export tests inject errors in-process, not actual power failures.

Checking a backup is read-only. It authenticates with that copy's password,
requires the selected network and complete current wallet address, and compares
the full decoded state. An authentic same-wallet copy may differ from current
state; the UI explicitly distinguishes this from an exact match. Different-wallet,
wrong-network, damaged, oversized, symbolic-linked and hard-linked input is refused.
The check never imports, resets scan height, replaces pending transactions or
broadcasts. It proves neither chain validity nor current balance/freshness.

Both operations run under the existing background wallet ownership/pause gate.
Focus loss during work requests a lock before republishing the wallet. Entered
path/password and acknowledgement are cleared on submission or concealment. Old
copies retain their old passwords after password rotation, even when the complete
decoded wallet state still matches. Backups cover seed/outputs/history/reservations,
not contacts, preferences or chain files.

The development UI currently takes absolute filenames; native file-picker polish
and actual removable-device/platform testing remain open. Full-state import of
old snapshots is still disabled: stale pending transactions need an explicit
reconciliation/quarantine policy before any future automatic resend path.

### Explicit keys-only recovery from encrypted backups

First-run setup and an empty interrupted enrollment now offer keys-only recovery
from a `.nfv` backup. A read-only background operation checks the bounded regular
file, password, authenticated selected network and wallet payload. It returns only
the keys into a fresh in-memory wallet, with birth/scan height zero and empty
outputs/history/reservations. No source raw transaction becomes resendable in
the recovered wallet. The original file is not changed or deleted.

Before writing, Core displays the complete derived address and source scan/history/
pending/reservation counts. These counts are local metadata, not chain validation.
The user must explicitly confirm the address and keys-only limitation and choose
a new Vault password with confirmation. Existing encrypted-first provisioning
then commits the fresh wallet and ends locked; a separate unlock starts the node.
No plaintext seed is saved. Existing wallets, damaged snapshots and retired seed
markers are not bypassed or overwritten by this recovery option.

This does NOT restore original outgoing history, pending sends, receipts or
reservations, and it does not cancel already broadcast transactions. Keep the
source backup and review prior payments before spending. A new
scan reconstructs what the chain makes discoverable, not every local annotation
or original per-transaction record. A successful password check alone does not
prove that this is the wallet the user intended; the address preview is essential.

Preview decryption runs off the UI thread and writes no wallet files. Focus loss,
hidden windows, five minutes of inactivity and explicit cancellation clear input
and drop preview secrets. A running KDF is allowed to finish privately; its result
is discarded if concealment occurred. No secret phrase is displayed or copied to
the clipboard during backup-key recovery. Application buffers are zeroizing, not
a promise of erasing egui/OS copies. Six new local tests cover source preservation,
recovered empty state across restart, invalid input, late result discard, new
password/confirmation, locked restart without node and narrow-window layout.

## Recovery Studio

Rehearsal compares the complete public address derived from the entered words
against the selected wallet. A valid BIP39 checksum alone is insufficient.
It never creates a wallet directory, replaces an active wallet or sends data.
Wrong-wallet errors do not reveal the candidate address or echo the phrase.
Core clears the entered words after every attempt, on cancellation, loss of
focus and leaving Settings. Contacts and preferences are explicitly outside the
coverage of a successful phrase check.

## Verification

Tests cover lock gates, exact state preservation, ciphertext/header/tag
tampering, wrong passwords/networks, invalid authenticated payloads, random
nonce changes, password rotation, strict legacy parsing and recovery mismatch.
The fixed Argon2id test vector is independently computed with Python
cryptography/OpenSSL, using a public test password and salt.

Native/WASM interoperability uses `vault_interop` and
`scripts/check-vault-wasm.cjs`, with an unfunded public zero-seed test wallet.
Fixtures are written only to newly created test paths, never production wallet
directories. Passing tests is not independent cryptographic review.
