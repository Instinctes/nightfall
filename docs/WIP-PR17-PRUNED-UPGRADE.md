# Existing 1.1.x pruned datadirs: safe upgrade boundary

This is an **unfinished migration design**, not a validated automatic migration.

Version 1.1.2 pruned datadirs may not carry the semantic integrity seals
introduced by PR #17. Their old block bodies are no longer available.
Hashing the remaining local files **cannot reconstruct independent proof**
that those files describe a valid historical chain.

## Safe current behavior

- Fail closed without rewriting chain metadata or any persisted state.
- Never automatically calculate and bless new local seals from unverified
  legacy pruned contents. This would turn local corruption into trusted state.
- Back up the datadir and wallet before doing anything.
- Retain the wallet, explicitly resync chain history from independently
  verified archive bodies, then write new local seals only after validation.
- On failure/interruption, refuse startup and retain the original backup.

The regression in this WIP branch asserts byte-for-byte preservation
of old metadata, horizon, headers, and retained bodies on refused startup.

## Still required before merging

A real untouched 1.1.2 pruned fixture and tested recovery UX, an explicit
safe upgrade path, interruption fault injection, mainnet-scale startup
benchmark, and maintainer approval after the consensus activation freeze.

## Explicit offline Node Kit migration command (WIP)

After stopping the node and backing up the wallet, the operator can run:

```shell
nightfall-nodekit --network mainnet upgrade-pruned --datadir /ABSOLUTE/PATH/TO/N8
```

Without `--force` the command refuses and does not touch wallet or chain.
To perform the confirmed archive backup and chain reset:

```shell
nightfall-nodekit --network mainnet upgrade-pruned --datadir /ABSOLUTE/PATH/TO/N8 --force
```

This command checks that the datadir is a legacy pruned installation lacking
complete seals, requires an explicit network and path, takes the one-writer
directory lock, and invokes the existing journaled `recover-stalled-fork`
implementation. It leaves the wallet untouched and preserves the old chain
files under a recovery backup. The restarted daemon must independently
resynchronize the blockchain; no deleted historical block bodies are
fabricated or implicitly considered verified.

This is a destructive **chain-state** reset, not a trust-preserving in-place
seal migration. The untouched 1.1.2 fixture, benchmark and runtime tests
remain **open** until executed successfully and reviewed by the maintainer.
