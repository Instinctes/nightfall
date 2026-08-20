# How NIGHT gets a price

Nobody sets a price. Not the inventor, not the website, not this file.

A price is two strangers disagreeing: one wants NIGHT, one will part with
it. Until that happens, the only number that exists is what it **costs to
mine** a block on your own hardware.

## What exists today

- **Mine it.** Core → Start mining. Reward is 6 NIGHT per block, ~15 s
  target, CPU (Nighthash-v2 / Argon2id). There is no pool and no premine.
- **Receive it.** Someone already mining can send you `nf1…`. The
  browser wallet at https://nightfallcoin.org/wallet/ makes an address
  without installing anything. Same 24 words as Core.
- **No official ticker.** Any EUR/USD figure you see that did not come
  from a trade you made is fiction.

## How a market actually forms

1. A miner sells some coins peer-to-peer (forum, OTC, later Bisq-class).
2. The same pair trades more than once. That is a range, not a listing.
3. A book appears somewhere that will touch a privacy coin. Then outsiders
   can quote a number. That number is still only as real as the last fill.

Wash volume, a website ticker, or “market cap = 90 million × wish” does
not create value. It creates a target.

## Why distribution matters more than marketing

If two machines mine most of the blocks, the float is not a market. It is
an inventory. Privacy-coin buyers look at that first.

The protocol will not stop you mining. The social contract of a fair
launch will. Leave blocks for other CPUs.

## After the last reward

Fees stop being burned and go to the miner once the subsidy runs out.
That takes 30 halvings — 225,000,000 blocks, roughly 107 years — and the
curve terminates at 89,999,999.25 NIGHT, three quarters of a coin short
of the cap, because every halving truncates to whole darks.

The practical horizon is much nearer: 89 million are minted in about
23.4 years, so the block reward is a rounding error long before it
formally reaches zero. Security then depends on fees, and therefore on
people who want the chain to exist. See `docs/MAINNET.md`.
