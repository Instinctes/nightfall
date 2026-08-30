# NIGHT ↔ BTC swap — what you can lose

**Not for real coins.** The mainnet gate stays closed until the cross-curve
proof is reviewed outside this project. This page is the loss list the
interface has to name, not a promise that any of it is safe.

There is no NIGHT refund. v0.1 claimed one; it was broken; it is withdrawn.

## While nothing is locked

A restart, a lost packet, a mistyped address, a cancelled offer. Nothing
moves on either chain. Harmless.

Secrets for the handshake live in `{datadir}/swaps/{id}.secret` and must be
mode `0600`. A world-readable file is refused on load. Losing that file after
the Bitcoin lock is the same as losing the spend key.

## After Bob locks Bitcoin, before Alice locks NIGHT

Bob's BTC sits in a 2-of-2 until `H₁`. Then he cancels and refunds. He waits;
he does not lose the coin unless he never broadcasts the abort. Alice has
lost nothing.

## After both locks confirm

| What happens | Bitcoin | NIGHT |
|---|---|---|
| Alice redeems in time, Bob claims | Alice has it | Bob has it |
| Alice never redeems | Bob cancels, refunds | If Alice locked, she can claim once Bob refunds (`s_b` is published). If Bob never refunds, she punishes at `H₂` and **the NIGHT is stuck forever**. |
| Alice redeems too close to `H₁` | Bob cancels; the redeem does not confirm; he refunds | `s_a` is already public (mempool, logs, a reorged block). Bob claims NIGHT. **He has both.** |
| A crash while Alice is redeeming | Same as the row above if a restart cancels | Same. The wallet must not offer Cancel in `Redeeming`. |
| Reorg of the NIGHT lock after Alice redeemed | Alice keeps BTC | The lock disappears; Bob cannot claim. Alice has both. Waiting `MAX_REORG_DEPTH` (500) NIGHT blocks before redeeming is the defence. Waiting less is accepting this. |
| Reorg of TX_redeem deeper than 6 | The BTC lock is unspent again | `s_a` may already have been used. Residual, out of scope without majority hashrate. |

The row in bold is spec §9.2. It is the reason the state machine refuses to
redeem inside the margin, and the reason a Bitcoin outage is **not** treated
as "0 confirmations".

## What this wallet will not do

- Refund NIGHT. There is no such transaction.
- Talk to a mailbox. A server that can withhold one packet is an operator.
- Taproot. The abort tree is P2WSH 2-of-2, ECDSA.
- Start a swap on mainnet.

## If a packet is lost

Copy it again. Sequence numbers do not wrap and a replay of an old one is
refused. After a restart the last packet we produced is still on disk.

## If the secret file is gone

After the Bitcoin lock, the swap is over for this wallet. There is no seed
derivation that rebuilds `s_a` or the Bitcoin 2-of-2 key. They were sampled
for this swap only.
