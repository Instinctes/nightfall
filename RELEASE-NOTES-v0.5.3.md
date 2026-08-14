Same chain as v0.5.0 — protocol v7, genesis `c8614333…`. **Upgrade in place; no
reset, no reindex.** Every fix below is in the networking and wallet layer.
Consensus, block format and genesis are untouched.

## Four faults, all introduced by the v0.5.1 networking work

The v0.5.1 and v0.5.2 releases parallelised peer sync and block announcement.
That work was correct in outline and wrong in four specific places. Each of
these was found by re-reading the diff rather than by a test, which is the
honest summary of where this project's test coverage currently ends.

**`connect_peer` had no connect timeout.** `TcpStream::connect` waits for the
OS SYN retry — often more than a minute — on any address that does not answer.
The sync loop joins every peer thread before starting the next round, so one
unreachable address in `peers.json` stalled the entire round for all peers.
Now `connect_timeout` is applied per resolved address.

**Known peers were written but never dialled.** `peers.json` was persisted on
every change and reloaded on start into a set that the dialler never consulted;
`peer_addrs` began each run empty. A node that had met the whole network still
restarted knowing nobody, and depended entirely on the seed nodes.

**The behind-guard was disabled from boot.** `behind_since` was initialised to
`0`, so the "have we been behind for too long" comparison measured against the
epoch and never fired. The catch-up path it guards therefore never ran.

**`reconcile_with` could delete live outputs.** The guard for "does this block
range cover the canonical history" was `blocks.first().height == Some(0)` — true
for any page that happens to start at genesis, including a 128-block CLI page.
Reconciling against such a page dropped every output above height 128 as though
the chain no longer held it. Replaced by `covers_canonical_history`, which
checks the range actually spans from the wallet's birth height to its scan tip.

Credit where it is due: all four were found by an outside review of the diff,
not by me and not by the test suite.

## What this does not fix

None of the 138 tests start two nodes. This entire class of fault — peers,
timeouts, sync ordering — is invisible to the suite before deployment, and
that is why these bugs shipped. A multi-node integration harness is the next
piece of work, ahead of further networking changes.

## Verified before release

`cargo fmt --check` clean · `clippy --workspace --all-targets -D warnings`
clean · 23 suites, 138 tests passing.

Convergence measured on the live network after deploying this build: a MacBook
and the Frankfurt seed node tracked each other within ±2 blocks over five
consecutive samples, with the laptop periodically ahead — so it both follows
and propagates.

## Downloads

`NIGHTFALLCOIN-Core-0.5.3-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.5.3-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.5.3-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.5.3-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.5.3-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.5.3.txt` and `SHA256SUMS-0.5.3-windows.txt`.
The Windows binaries are built by
[a published workflow](.github/workflows/release.yml) on GitHub's runners.
