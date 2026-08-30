**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. No consensus
change. 0.8.x peers with this normally.

**Upgrade if you swap or mine on Windows.** Experimental NIGHT↔BTC swaps
land here, still **disabled on mainnet**. A Windows close-to-tray default
that left extra processes running is unchanged in behaviour: Quit from the
tray, do not open a second Core on the same folder.

## Experimental NIGHT ↔ BTC swaps

Wallet-to-wallet, copy-paste packets, no operator. Bitcoin is P2WSH 2-of-2
with an ECDSA adaptor; NIGHT is a shared stealth address. There is **no
NIGHT refund**. If the other side cancels and never refunds, NIGHT locked
in the swap is stuck forever.

Mainnet will not start a swap. Testnet and devnet will. The cross-curve
proof uses our own Ristretto leaf inside a reviewed combinator crate.
Nobody outside this project has signed that leaf. The gate stays until
it is opened on purpose.

Operator notes: `docs/SWAP.md`. Loss cases: `docs/SWAP-LOSS.md`. What we
attacked: `docs/SWAP-ATTACKS.md`.

bitcoind talking to the wallet needs `-txindex=1`. Without it, confirmed
locks look missing and the swap goes blind while H₁ runs.

## Driver

A redeem that was safe last tick is not safe forever. Between ticks the
Bitcoin chain moves. The driver re-reads depth every time and will not
re-broadcast a pending redeem inside the H₁ margin. That race is how the
other side takes both coins.

## Builds

This tag is 0.9.0 in the tree. Binaries, checksums, website, seed
installs: built and shipped by the operator, not from this checkout.

fmt, clippy `-D warnings`, workspace tests, mutation stand.
