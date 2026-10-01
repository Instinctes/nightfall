# Privacy — what is true, what is not

Nightfall hides **amounts** and **addresses**. It does not erase the
transaction graph, and it does not hide you from the first peer unless you
use the tools below.

## On the chain

| Property | Status |
|---|---|
| Amounts | Hidden (Pedersen + Bulletproofs) |
| Recipient address | Never appears (`nf1` → one-time key) |
| Sender address | Never appears |
| Transparent pool | None — privacy is mandatory |
| View keys | `nfview1…` sees amounts/memos, cannot spend |
| Payment receipt | Prove **one** output without the view key |
| Graph | Obscured by block aggregation, **not erased** |
| Cut-through | Not applied (would break one-sided payments) |

## On the network

| Property | Status |
|---|---|
| Dandelion++ stem/fluff | **On for negotiated peers.** Transactions use epoch-stable routes, a separate non-mineable stempool, bounded route repair and randomized fail-safe embargo before diffusion. |
| Legacy peers | Stem capability is negotiated explicitly. Legacy sessions are not selected as Dandelion stem routes; legacy transaction messages remain fluff-compatible. |
| Tor / SOCKS5 | **Default** (`127.0.0.1:9050`). If Tor is down, clearnet fallback. `.onion` never falls back. `NIGHTFALL_PROXY=off` disables. |
| Dandelion++ + Tor | They protect against different observers: Dandelion++ reduces origin exposure to relay peers; Tor/SOCKS5 hides outbound peer destinations from the local network path. |

### Tor

```bash
# nightfalld
nightfalld --network mainnet run --proxy 127.0.0.1:9050

# or
export NIGHTFALL_PROXY=127.0.0.1:9050
```

In Core: Network → SOCKS5 / Tor → `127.0.0.1:9050` → Apply.

A `.onion` seed is accepted only while the proxy is on (otherwise the
system resolver would be asked for it).

## Selective disclosure

```bash
# one payment, not the whole wallet
nightfall-wallet --network mainnet prove-output --commit <commit hex>
nightfall-wallet --network mainnet export-receipt --txid <txid>
nightfall-wallet --network mainnet verify-receipt --file receipt.json
```

A received/mined receipt opens the commitment (amount + blinding) and is
signed by the spend key of `nf1…`. An auditor checks the opening and the
signature. They still need a node to confirm that commitment is on chain.

The view key remains the tool for “see everything.” A receipt is the tool
for “see this one payment.”

## Compared with Monero

| | Nightfall | Monero |
|---|---|---|
| Amounts hidden | yes | yes |
| Addresses off-chain | yes | yes (stealth) |
| Supply independently proven | **yes** — `Σ UTXO − Σ excess = (minted − burned)·G` | no (ring-size / decoy assumptions) |
| Mandatory privacy | yes | yes |
| Network-layer default | Dandelion++ + optional Tor | Dandelion++ |
| Anonymity set today | every tx **in the same block**; network is small | protocol-scale, years of usage |
| Independent audit | not yet | multiple |

Do not claim “untraceable” or “protocol-scale anonymity.” The honest line:
**private amounts, no addresses on chain, a supply anyone can prove, a
graph that is mixed per block, a first hop that is no longer a broadcast.**
