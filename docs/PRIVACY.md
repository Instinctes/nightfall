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
| Dandelion-class stem/fluff | **On.** A new transaction is sent to **one** random peer first, then fluffed. |
| First hop to an old node | That node broadcasts. Stem only helps when the next hop also stems. |
| Tor / SOCKS5 | **Default** (`127.0.0.1:9050`). If Tor is down, clearnet fallback — unless `NIGHTFALL_PRIVACY_LOCKDOWN=1` (below). `.onion` never falls back. `NIGHTFALL_PROXY=off` disables. |
| Dandelion++ fluff via Tor | Combine both. Stem hides the origin from peers; Tor hides the peers from the ISP. |

### Privacy lockdown: fail closed when Tor is down

By default, a node whose Tor proxy is down connects to peers **directly**,
which shows your IP address to them. To refuse that instead, set one
environment variable before starting:

```bash
NIGHTFALL_PRIVACY_LOCKDOWN=1
```

With it set:

- Every new outgoing P2P connection must go through SOCKS5. If the proxy is
  missing, unreachable or refuses the connection, the dial fails — there is
  no direct fallback.
- The node does not fetch the HTTPS peer directory (that request is direct).
- Built-in seeds are still reached, through SOCKS5.
- A manually added peer is refused unless a proxy is configured (resolving
  its name would otherwise go to your DNS resolver).
- Core's "check against nightfallcoin.org" is refused (it is a direct HTTPS
  request).

`0`, `false`, `off` and an empty value leave it off. Any other value turns it
on — a typo must not silently allow clearnet. Restart after changing it;
connections that are already open are not closed.

Use it with a working proxy, e.g. `--proxy 127.0.0.1:9050` and Tor running.
Lockdown with the proxy set to `off` means no outbound P2P at all.

Setting it for Core, which reads it at start:

```bash
# macOS (start from Terminal; Finder does not pass the variable)
NIGHTFALL_PRIVACY_LOCKDOWN=1 "/Applications/NIGHTFALLCOIN Core.app/Contents/MacOS/nightfall-core"

# Linux
NIGHTFALL_PRIVACY_LOCKDOWN=1 ./nightfall-core-<version>-linux-x64
```

```bat
:: Windows (Command Prompt)
set NIGHTFALL_PRIVACY_LOCKDOWN=1
nightfall-core-<version>-windows-x64.exe
```

For `nightfalld` under systemd: `Environment=NIGHTFALL_PRIVACY_LOCKDOWN=1` in
the unit.

**Not covered:** inbound connections (peers that dial you see your address —
do not open a listening port if that matters), the browser wallet, other
programs and the operating system. This is not a complete anonymity or
traffic-isolation guarantee.

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
| Network-layer default | stem/fluff + optional Tor | Dandelion++ |
| Anonymity set today | every tx **in the same block**; network is small | protocol-scale, years of usage |
| Independent audit | not yet | multiple |

Do not claim “untraceable” or “protocol-scale anonymity.” The honest line:
**private amounts, no addresses on chain, a supply anyone can prove, a
graph that is mixed per block, a first hop that is no longer a broadcast.**
