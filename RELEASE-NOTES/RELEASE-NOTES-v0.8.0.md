**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. 0.7.x still
peers. Nothing here changes consensus, so nobody is forced to upgrade —
but see **Do not downgrade afterwards** before you do.

## The chain file was three quarters air

`serde_json` renders a `[u8; 32]` as an array of decimal numbers —
`[241,118,175,…]` — so every byte of every hash, commitment, signature
and range proof cost about 3.6 characters on disk. Measured on mainnet
at 31,288 blocks: **152.3 MiB, 5.10 KiB per block**, for a chain in
which almost every block carries one output and one kernel. Over a year
of 15-second blocks that is 10.7 GB of an *empty* chain, before anybody
sends anything.

The same chain in a compact binary encoding is **42.2 MiB — 1.41 KiB per
block, 72.3 % less disk**. Same tip, same UTXO root, same supply proof.
The conversion takes about a second.

This is a disk layout and nothing more. Block hashes are computed over
raw field bytes, never over the serialised form, and P2P messages stay
newline-delimited JSON. A converted node and an unconverted node hold
the identical chain and talk to each other exactly as before.

**Nothing converts by itself.** Existing nodes keep their `blocks.jsonl`
until you ask:

```
nightfalld --network mainnet migrate-storage
```

It writes `blocks.bin.tmp`, reads it back, compares **every block hash**
against the source, and only then swaps the files, keeping the old one
as `blocks.jsonl.pre-binary`. Stop the node first — there is no lock on
the chain file. A **new** install starts binary; there is no history to
stay compatible with.

## Pruned nodes

`--prune`, or Settings → **Prune old blocks**. Drops block bodies older
than the 500-block reorg window and keeps the UTXO set, the headers from
genesis, and the supply invariant. A pruned node still validates every
new block in full. It cannot serve initial sync below its horizon,
cannot rescan stealth outputs from genesis, and cannot export a
snapshot, so seeds and anything running `--mobile-listen` stay archives.

Combined with the binary format, a pruned node is a small fraction of
what a full archive was, and it is still a full validator.

## Getting ready for a crowd

Three limits found by working the numbers against a million miners
rather than waiting to meet them.

**Introducer mode**, `--introducer`. A seed does two unrelated jobs and
only relaying needs a socket held open. Introducing is one round trip:
hello, here is who else answers, goodbye. Holding introductions as live
sessions is what capped a seed at its peer limit — two seeds at 128
seats were 256 slots for the entire network. Hanging up before the
session pool is touched turns the same machine into thousands of
introductions per second.

**Checkpoints.** A pin at height 25,000, cross-checked byte for byte on
two independent machines before it went in. It bounds how far a chain
can be rewritten and lets a fresh node skip re-proving history it can
verify by hash. `NIGHTFALL_NO_ASSUME_VALID=1` verifies everything
anyway.

**Archives first.** Nodes now say whether they are pruned in the
handshake, and a node that is behind dials archives before it dials
anyone else, instead of asking a pruned peer for blocks it does not
have.

The difficulty algorithm needed nothing: simulated against the real
LWMA, a 5,942× hashrate jump settles in 523 blocks over 22 minutes.

**Light wallet failover.** `/wallet-api`, `/supply`, `/peers` and
`/network.json` went through a single hostname, so one VPS incident took
every phone and browser wallet with it. Now a list is tried in order,
6-second timeout, and a 5xx counts as a failure — a proxy faithfully
relaying 502 from a node that is still syncing is correct and useless.

## Do not downgrade afterwards

Once a datadir is binary, a 0.7.x node cannot read it. It finds no
`blocks.jsonl`, concludes the datadir is empty, and downloads the chain
again from genesis. Your keys are safe and the chain is unaffected — you
just lose the hours.

So: upgrade when you want, but treat it as one-way. If you need to go
back, delete `blocks.bin` first and rename `blocks.jsonl.pre-binary`
back to `blocks.jsonl`; that file is kept for exactly this reason.

## Fixed

- Reorg-time rewrites wrote JSON regardless of the datadir's format,
  which corrupted the chain file of a converted node on its first deep
  reorg. Found and fixed before release; the recovery path is the
  `.pre-binary` file above.
- Snapshots now carry their format in the file name and in
  `snapshot.json`, and the importer converts instead of copying bytes
  across formats.
- A JSON file in a binary datadir now says so, instead of reporting that
  block 0 claims 1.7 GB.
- `resync-chain` keeps the datadir's storage format and names its backup
  after what is actually in it.

210 tests, fmt and clippy clean.

Phone and browser wallets stay 0.7.0 — the light client keeps no chain
file, so none of this touches it.

## Downloads

`NIGHTFALLCOIN-Core-0.8.0-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.8.0-macOS-intel.dmg`
`nightfall-core-0.8.0-windows-x64.exe`
`nightfalld-0.8.0-windows-x64.exe`
`nightfall-wallet-0.8.0-windows-x64.exe`
`nightfall-core-0.8.0-linux-x64`
`nightfalld-0.8.0-linux-x64`
`nightfall-wallet-0.8.0-linux-x64`

Verify `SHA256SUMS-0.8.0.txt`, `SHA256SUMS-0.8.0-windows.txt`,
`SHA256SUMS-0.8.0-linux.txt`.
