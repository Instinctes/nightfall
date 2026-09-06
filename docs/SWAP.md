# NIGHT ↔ BTC atomic swap — operator notes (v0.9.5)

**Experimental. Not for real coins on mainnet.** The wallet will not start a
swap on mainnet. Testnet and devnet are open.

Protocol: [`SWAP-SPEC-DRAFT.md`](SWAP-SPEC-DRAFT.md) v0.3. Loss cases:
[`SWAP-LOSS.md`](SWAP-LOSS.md). What we attacked and what we did not:
[`SWAP-ATTACKS.md`](SWAP-ATTACKS.md).

## What this is

Two people who already agreed on a price exchange NIGHT for BTC without an
operator. Copy-paste packets. No mailbox, no server, no independent timed NIGHT refund.
Alice can recover NIGHT after Bob publishes his Bitcoin refund; otherwise it
can remain locked permanently.

Bitcoin side: P2WSH 2-of-2, ECDSA adaptor, CSV abort tree (lock / redeem /
cancel / refund / punish). NIGHT side: shared stealth address, spend
`s_a + s_b + offset`. Adaptor lives on Bitcoin only.

## What you need

- NIGHTFALL Core 0.9.5 on **testnet** or **devnet**
- bitcoind (same network as the swap) with **`-txindex=1`**, credentials in
  `{datadir}/bitcoin-rpc.conf` (mode 0600: `url=`, `user=`, `password=`)
- Three Bitcoin addresses from *your* Bitcoin wallet (refund, redeem, punish)
- The counterparty, reached by whatever channel you already trust

## What you must not do

- Run two Core processes on the same datadir
- Redeem close to H₁ (the wallet refuses; do not override it)
- Reuse a swap share, Bitcoin 2-of-2 key, or scan secret
- Expect an independent timed NIGHT refund. If the other side cancels and never
  refunds, NIGHT locked in the swap is stuck forever.

## Public test phase

Use testnet. Report:

1. The exact error string (not a screenshot of a toast)
2. Both swap ids, both roles
3. `status` from your node (tip, peers, loading)
4. Whether more than one `nightfall-core` process was running

Do not send seed phrases. Do not send `.secret` files.

## Builds

This tree is 0.9.5, a regular mainnet wallet release with swaps still gated on
mainnet. Use explicit `--network devnet` or `--network testnet` for swap testing.
See the [release notes](../RELEASE-NOTES/RELEASE-NOTES-v0.9.5.md).
Production seed services are not upgraded by this wallet release.
