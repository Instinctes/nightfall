# Offline stalled-fork recovery (headless nodes)

`nightfalld --network mainnet --datadir /EXISTING/CHAIN/DIRECTORY recover-stalled-fork --force`

Only run after stopping **all** processes holding that data directory. The CLI takes the same OS-level lock as the wallet/node and refuses a live holder. Network, existing datadir and `--force` are mandatory. The command does not mine, connect to peers, contact RPC, change consensus, reset wallets, modify keys or remove earlier backups.

The operation verifies the existing chain/network, stages a genesis replacement **in the same binary or JSON format**, reads it back, fsyncs it, allocates a distinct backup directory and records a transaction journal. It moves chain bodies, metadata, and (for a pruned store) headers and horizon to the backup, then installs the staged fresh bodies and metadata. Each failure after moving a file attempts a reverse-order rollback. It never reports success unless the installed genesis is verified. It rejects linked/nonregular chain files, mixed/ambiguous encodings, absent/inconsistent metadata, wrong networks, incomplete pruning sidecars and unfinished prior transactions. Other data-directory files, wallet files, install ID and pre-existing backups are untouched.

## Next step after success

Copy the printed backup path somewhere safe. Start the node in `run` mode with the *same* explicit `--network` and `--datadir`, **without `--mine`**, and wait for a complete catch-up against healthy peers. Do not resume mining until synchronized; the existing catch-up mining guard is unchanged. This escape hatch does not promise to cure the underlying liveness issue.

## Power loss or incomplete rollback

A crash between individual renames can leave `.stalled-fork-recovery-in-progress` behind. **Do not restart the node/wallet or invoke recovery again.** Stop all processes using the directory. Read the journal's `BACKUP`, `STAGE`, and `FILES`; preserve the directory and a complete copy of its current state first. Under the same offline data-directory lock, move any partially installed genesis files into a separate quarantined directory, then restore **each original chain file named in `FILES`** from `BACKUP` to its original location. Keep `install-id`, wallet/key files, all earlier backups and all other data-directory contents unchanged. Compare restored files byte-for-byte with a copy of the saved backup, and confirm the original tip loads correctly before deleting the journal. On any ambiguity or missing original file, stop and seek review rather than guessing. A power-loss mid-transaction is recoverable from the backup/journal but **not automatically crash-atomic**.

## Development test gates

The offline unit tests cover JSON/binary, a genuine pruned UTXO horizon, backup restore to the exact original tip, wallet file identity, unique repeat backups, wrong-network refusal, symlinks, incomplete journal and debug-only injected staging/move/verify failures. The integration test uses the real CLI and the shared OS filesystem lock to simulate an active node/wallet holder; the lock policy itself is identical for both applications. Debug-only fault injection is not compiled into release builds.
