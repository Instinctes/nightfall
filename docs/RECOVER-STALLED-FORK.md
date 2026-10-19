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

- Backs up the current chain files into a **freshly created, uniquely named**
  directory inside the datadir (`recovery-backup-<nanos>-<n>`), and never
  deletes or reuses a backup.
- **Stages and verifies** a fresh genesis before moving a single original file,
  so a failure leaves the originals recoverable rather than a half-reset datadir
  reported as success.
- Resets the datadir to a fresh genesis, **preserving the storage format**
  (binary or JSON) and the chain metadata / headers / pruned-UTXO-horizon file
  set.
- **Never touches** the wallet directory, `miner.seed`, the install id, the
  lock file, or any earlier backup.

## Safety boundary

- **Offline only.** The command takes the one-writer data-directory lock (the
  same lock a running node or a Core wallet holds), so it refuses rather than
  race a live writer. There is no recovery RPC and no automatic mining.
- Requires an explicit `--network`, an existing `--datadir`, and `--force`.
- Refuses an ambiguous datadir — no readable `chain-meta.json`, a symlinked or
  non-regular chain file, or a chain for a different network — rather than
  guessing.
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

If anything goes wrong mid-reset, the command says so and does **not** report
success; your original chain files are in the backup directory it named —
move them back into the datadir to restore the previous state.
