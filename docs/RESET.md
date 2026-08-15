# Reset to protocol v8 (executed)

Decision **B** from the restart draft: new genesis, slower emission, fees
to the miner after the subsidy, Tor by default, public history.

## What changed

| | Abandoned v7 | This chain (v8 / n8) |
|---|---|---|
| Protocol | 7 | **8** |
| Wire | 5 | **6** |
| Magic | `NFL1` | **`NFL2`** |
| Genesis | `c8614333…` | **`061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de`** |
| Datadir | `nightfall/<network>/` | `nightfall/<network>/n8/` |
| Era-0 reward | 20 NIGHT | **6 NIGHT** |
| Halving | 2,250,000 blocks (~1.07 y) | **7,500,000 blocks (~3.56 y)** |
| 89 M minted | ~7 years | **~23.5 years** |
| Terminal supply | 89,999,999.7075 | **89,999,999.25** (0.75 NIGHT short of the cap) |
| Fees while subsidy > 0 | burned | **burned** |
| Fees after subsidy | burned (miner unpaid) | **paid to the miner**, not minted, not burned |
| Tor | optional | **default** (`127.0.0.1:9050`), clearnet fallback |

`2 × 6 × 7,500,000 = 90,000,000`. Same ceiling, different clock.

## What we do not claim

- Not 100 % anonymous.
- Not untraceable.
- Not a protocol-scale anonymity set. The set is the other transactions
  in the same block, plus however many people actually use the chain.
- No official price. No listing. No premine. No company.

See [PRIVACY.md](PRIVACY.md) and [GETTING-NIGHT.md](GETTING-NIGHT.md).

## Operator notes

Old 0.6.x data stays where it is. A new install writes `n8`. Delete the
old folder only if you are sure you do not want the archive.

```
# macOS
~/Library/Application Support/nightfall/mainnet/       # v7 archive
~/Library/Application Support/nightfall/mainnet/n8/    # this chain
```

```
nightfalld --network mainnet run            # Tor default
nightfalld --network mainnet run --proxy off
```

Seed: `seed.nightfallcoin.org:17891`. Must run 0.7.0+. An old seed speaks
`NFL1` and will never complete a handshake.
