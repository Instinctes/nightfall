# Recovering a headless node stuck on a dead fork

`nightfalld recover-stalled-fork` is the offline escape hatch for a node that
has ended up permanently stalled on a branch it can never leave — the failure
behind [#40](https://github.com/Instinctes/nightfall/issues/40): hours at one
height, no recovery path, `mining:true` the whole time.

1.0.6 fixes the root causes of that stall, and NIGHTFALLCOIN **Core** offers
**Resync chain, keep wallet** from its dashboard for the rare case it still
happens. A headless `nightfalld` — a systemd service, a node on WSL2, a mining
rig — has no dashboard, and the only move left was to delete chain files by
hand. This command is that move, made safe and explicit.

## What it does

- Validates its inputs **before** taking the data-directory lock, so a bad
  `--datadir` is refused without being created and a symlinked lock file is
  never written through.
- Writes a **recovery journal** (`.recovery-journal.json`) before it moves
  anything, and removes it only once the reset is complete and verified. While
  the journal exists, every load path — the daemon, Core, and the CLI — refuses
  to start, so an interrupted reset can never be mistaken for a healthy fresh
  chain. The journal is created **exclusively** (`create_new`), and both the
  writer and the shared start-up guard inspect the path with `symlink_metadata`,
  so a journal that is a symlink or any other non-regular file is refused and
  **never followed** to an external target.
- Backs the current chain files up into a **freshly created, uniquely named**
  directory (`recovery-backup-<nanos>-<n>`), fsyncing at each step, and never
  deletes or reuses a backup.
- **Stages and verifies** a fresh genesis before moving a single original file;
  on any mid-reset failure it names the backup and does **not** report success.
- Resets the datadir to a fresh genesis, **preserving the storage format**
  (binary or JSON) and the chain metadata / headers / pruned-UTXO-horizon file
  set.
- **Never touches** the wallet directory, `miner.seed`, the install id, the
  lock file, the journal, or any earlier backup.

## Safety boundary

- **Offline only.** The command takes the one-writer data-directory lock (the
  same lock a running node or a Core wallet holds), so it refuses rather than
  race a live writer. There is no recovery RPC and no automatic mining.
- Requires an explicit `--network`, an existing `--datadir`, and `--force`.
- Confirms the datadir's network **and genesis** from `chain-meta.json`. A
  valid recorded genesis is **required**: a missing, null or non-string
  `genesis_hash` is refused before anything is backed up or staged, so a reset
  never runs on a datadir whose identity cannot be confirmed. It also refuses an
  ambiguous datadir — a mixed binary/JSON layout, no readable `chain-meta.json`,
  a symlinked or non-regular chain file, or a chain for a different network —
  rather than guessing.
- This is a manual escape hatch, **not** a promise that every liveness problem
  is fixed. It resets to genesis; you resync.

## Use

Stop the node first, then:

```
nightfalld --datadir /var/lib/nightfall --network mainnet recover-stalled-fork --force
```

It prints the backup location and the next step. **Resync without mining
first** — start the node, let it reach the network tip, and only then start
mining. (The node's "don't mine while a fresh peer is ahead" guard enforces
this when you start with `--mine`, but starting without `--mine` until you are
at the tip is the clean path.)

```
nightfalld --datadir /var/lib/nightfall --network mainnet run --connect <peer>
```

## If a recovery is interrupted

If the command is interrupted mid-reset (power loss, a kill), it leaves the
`.recovery-journal.json` behind. The datadir may be only half-reset, so the
node, Core and the CLI will **refuse to start** and point you at the backup.
To recover by hand:

1. Move the chain files from the named `recovery-backup-<…>` directory back
   into the datadir.
2. Delete `.recovery-journal.json`.
3. Start the node normally.

Your original chain is intact in the backup, and the wallet was never touched.

## Durability and the platform note

Each file that matters is `sync_all`'d when it is written — the journal before
any originals move, the staged chain before it is installed — so file **contents**
are durable on every platform. The ordering of the directory entries (the
journal appearing, then the originals moving to the backup, then the staged files
appearing) is then made durable with a directory fsync at each boundary, and
any error from that fsync is **propagated**, not discarded: a sync failure
before the first move leaves the originals untouched and the journal in place, so
start-up refuses rather than reporting a half-reset chain as healthy.

Directory fsync differs by platform, and this is an honest limitation rather
than an equivalence:

- **Unix** fsyncs the directory handle directly; the error is surfaced.
- **Windows** has no user-space directory fsync. `FlushFileBuffers` on a
  directory handle returns `ERROR_ACCESS_DENIED`/`ERROR_INVALID_FUNCTION`, and
  NTFS instead orders directory-entry metadata through its own journal. The code
  recognises that specific result as the no-op it is (while still surfacing a
  real failure such as being unable to open the directory), so the
  rename-ordering guarantee is **weaker on Windows than on Unix**. File-content
  durability is unaffected because it comes from the per-file `sync_all`.

The data-directory lock's symlink refusal has the same shape of limitation:
`O_NOFOLLOW` makes it atomic on Unix, while on Windows the `symlink_metadata`
check is a best-effort guard with a TOCTOU window (an atomic refusal there would
need a `CreateFile` reparse-tag check). The lock lives inside an
operator-controlled datadir, so this is defence-in-depth, not a boundary against
a local attacker who can already write there.
