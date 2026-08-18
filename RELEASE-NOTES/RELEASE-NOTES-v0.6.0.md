**Update is required.** From this release the network speaks wire protocol v5.
Builds at 0.5.4 and earlier cannot complete a handshake: they will not sync,
will not propagate, and anything they mine lands on a branch no other node
accepts.

**This is not a chain reset.** Genesis is unchanged — `c8614333…` — the
protocol stays at v7, and every coin already mined is untouched and still
yours. An old wallet gets its balance back the moment it updates. A test pins
this: `raising_the_wire_version_did_not_move_the_genesis` fails the build if
the mainnet genesis hash ever moves, so an upgrade gate can never quietly
become a new chain.

## Why old builds are being refused

0.5.3 and earlier verify a chain reorganisation while holding the node's global
state lock. Measured on a live 1,737-block chain that takes 26.7 seconds, and
it grows with the chain. For that whole time the node answers no RPC, draws no
interface frame, serves no peer and accepts no block — then it resumes, mines
on a tip the network has already left, and forks.

That is not only the operator's problem. Every branch such a node produces is
work every other node has to reconcile. Refusing to peer with them is the only
lever this project has, and a small network cannot absorb the alternative.

The fix itself shipped in [v0.5.4](https://github.com/Instinctes/nightfall/releases/tag/v0.5.4).
This release makes it mandatory.

## Also in this release

**The handshake's version string is finally used.** Every node has always
announced what it runs and every node has always thrown that away, so the one
question worth asking during an incident — *what is everyone else running?* —
had no answer from anywhere. Peer versions are now recorded and reported:

```console
$ nightfalld --network mainnet status
peer_versions: { "nightfalld/0.6.0": 9 }
wire_version:  5
```

**The inbound handshake now checks the wire version.** Only the outbound path
did, so an old node was refused when we dialled it and served normally when it
dialled us. Refusing on one side is not refusing.

**The mining hold-off no longer gives up after 45 seconds.** That timeout was
chosen to escape a deadlock between two forked chains, at a time when a reorg
could block the node for half a minute on its own — so a perfectly healthy node
could reach it, start mining on a stale tip, and create the fork the timeout
existed to resolve. It is now ten minutes, and it measures time *without
progress* rather than time spent behind: a node that is catching up, however
far behind, never reaches it.

## Verified

143 tests passing. `cargo fmt --check` and
`clippy --workspace --all-targets -D warnings` clean on the toolchain CI uses.

## If you are on an old build

Download below, replace the app, and reopen it. Your wallet file, your seed and
your coins are where you left them — the chain has not changed underneath you.
If you keep the old build running it will report no peers, because there are
none that will speak to it.

## Downloads

`NIGHTFALLCOIN-Core-0.6.0-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.6.0-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.6.0-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.6.0-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.6.0-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.6.0.txt` and `SHA256SUMS-0.6.0-windows.txt`.
