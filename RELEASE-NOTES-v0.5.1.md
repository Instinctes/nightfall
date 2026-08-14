Same chain as v0.5.0 — protocol v7, genesis `c8614333…`. **Upgrade in place; no
data directory changes.**

## Nodes were falling ten blocks behind, and everything looked fine

Every connection succeeded, every log line was healthy, and nodes still drifted
behind the miner on the same network.

The sync loop walked its peers one after another. A peer that accepts a
connection and then fails costs the whole round its socket timeout, and every
peer queued behind it waits. After the v7 reset the peer list still held
addresses from the abandoned chain — reachable, listening, and unable to pass
the genesis check, five seconds each. A round took longer than the interval
between rounds, so the node that actually needed blocks was reached roughly
once a minute.

Measured before the fix: **one push in fifty seconds**, at an eight-second
interval.

Two changes. Each peer now gets its own thread, so one that hangs delays only
itself. And a peer that fails the genesis check is dropped from the list rather
than dialled again every round — after a reset the old network is still out
there, and there is no point paying for a handshake that cannot succeed.

## Downloads

`NIGHTFALLCOIN-Core-0.5.1-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.5.1-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.5.1-windows-x64.exe` · Windows 10+

Verify against `SHA256SUMS-0.5.1.txt` and `SHA256SUMS-0.5.1-windows.txt`.
Not code-signed: macOS right-click → **Open**; Windows **More info → Run anyway**.

---

**Website:** https://nightfallcoin.org
