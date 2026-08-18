# Nightfall 0.6.4 — stem, Tor, receipts

Same chain, same wire v5, same protocol v7. Older 0.6.1–0.6.3 nodes stay
peers. They just fluff a transaction the moment they see it.

## For people who care about privacy

- **Dandelion-class relay.** A locally created transaction is sent to one
  random peer first. Relays stay in that stem with 90 % probability and
  fluff to everyone after 12–28 seconds. Same `Tx` message as before.
- **SOCKS5 / Tor.** `--proxy 127.0.0.1:9050` on `nightfalld`, or
  `NIGHTFALL_PROXY`, or Core → Network. Destination hostnames are not
  resolved locally, so a seed lookup does not hit the ISP DNS.
- **Payment receipts.** Prove one received output (commitment opens, spend
  key signs) without handing over the view key. CLI:
  `prove-output`, `export-receipt`, `verify-receipt`. Core: Activity →
  Receipt.

## What this is not

- Not a hard fork. Emission is unchanged (20 NIGHT, 2,250,000-block
  halvings, 90 M cap).
- Not full Dandelion++. There is no separate stem graph. A hop onto a
  node that still broadcasts immediately is still a leak.
- Not an official price. None exists.

See `docs/PRIVACY.md` and `docs/GETTING-NIGHT.md`.
