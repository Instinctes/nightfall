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
| Tor / SOCKS5 | **Default** (`127.0.0.1:9050`). If Tor is down, clearnet fallback. `.onion` never falls back. `NIGHTFALL_PROXY=off` disables. |
| Dandelion++ fluff via Tor | Combine both. Stem hides the origin from peers; Tor hides the peers from the ISP. |

### Strict outbound P2P: fail closed

Set `NIGHTFALL_P2P_REQUIRE_PROXY=1` before starting
the node to require a SOCKS5 proxy for new outbound
P2P connections.

If the proxy is missing, unreachable or refuses
CONNECT, outbound P2P fails rather than connecting
directly. The normal fallback is unchanged when
strict mode is not enabled.

`0`, `false`, `off` and empty disable strict mode.
Other values enable it.

Use with a working proxy, for example
`NIGHTFALL_PROXY=127.0.0.1:9050`.
Setting the proxy to `off` while requiring it
disables outbound P2P.

This setting does not protect inbound P2P,
HTTP directory requests, browser wallet traffic
or other applications. Already-open connections
are not terminated. It is not a complete
anonymity guarantee.

### Sovereign privacy lockdown (experimental)

Set `NIGHTFALL_PRIVACY_LOCKDOWN=1` to enable
fail-closed outbound privacy protection.

Properties:

- New outgoing Nightfall P2P connections require SOCKS5.
- No direct P2P fallback when SOCKS5 fails.
- The direct HTTPS peer-directory bootstrap is disabled.
- Built-in P2P seeds remain accessible through SOCKS5.
- Manual peer entry cannot resolve hostnames without
  a configured proxy.
- The default networking policy remains unchanged.

Use `NIGHTFALL_PROXY=127.0.0.1:9050`
with a functioning SOCKS5 proxy.

When the proxy is unavailable, outbound P2P fails closed.

Restart the node when changing the environment.
Already-open connections are not terminated automatically.

This is not a complete anonymity or traffic-isolation
guarantee. Inbound connections, other applications,
browser-wallet requests and operating-system activity
remain outside its security boundary.

Unlike `NIGHTFALL_P2P_REQUIRE_PROXY=1`, this mode
also suppresses the node's known direct HTTP
peer-directory request.

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
