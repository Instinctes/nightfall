**Update if Core sits on an old tip while the seed keeps moving.** Same
chain, same genesis `061a052d…`, protocol v8, wire v6. 0.7.0 and 0.7.1
still peer. 0.7.2 is the build that leaves a one-block fork without
looking frozen, and that stops filling `peers.json` with Tor exits.

## A one-block fork looked like "sync is dead"

You mine (or accept) one block the seed does not have. The next pages
from the seed do not connect — `previous hash does not link to our tip`.
0.7.1 already measured rewind correctly (one block, not "too deep").
It then rebuilt the *entire* candidate from genesis, proof of work
included. Six thousand Argon2id checks is a minute of silence. The
wallet looks stuck. Mining, if the catch-up window has expired, extends
the bad tip and makes the next attempt worse.

Meanwhile `GetPeers` had written every gossip address to disk. After a
few sessions `peers.json` was sixty Tor relays on port 17891. Startup
opened a dial thread for each of them. Fifty sockets sat in `SYN_SENT`
while the seed, one hop away, had a single live session.

## The fix

Shared history is already ours. A reorg now re-applies that prefix
without hashing it again, and only verifies the suffix. A one-block
fork against a seed thirty blocks ahead is seconds, not a minute.

If a page does not connect and the peer is ahead, mining stops on that
tip until the reorg lands. That hold does not expire after ten minutes.

`peers.json` keeps compiled-in seeds and peers that actually completed
a handshake. Gossip stays in memory for the session. At most six
non-seed dials run at once; five failed connects drop the address.

Phone and browser wallets are unchanged. They talk to the seed over
the light API and never ran this path.

## Downloads

`NIGHTFALLCOIN-Core-0.7.2-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.7.2-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.7.2-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.7.2-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.7.2-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.7.2.txt` and `SHA256SUMS-0.7.2-windows.txt`.
Android / iOS stay on the 0.7.0 sideload builds.
