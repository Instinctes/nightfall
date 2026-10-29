# My Node — the web wallet on your own node

Status: shipped in 1.1.0 (Roadmap 2027, milestone 4).

The web wallet at wallet.nightfallcoin.org gets its chain data from the
project's seed node through nightfallcoin.org. *My Node* removes that
dependency: you serve the same wallet from your own domain, and it talks to
your own node. Nothing in the wallet's security policy is loosened to do it.

## What your node can and cannot do

The wallet is a light wallet. It does not download or verify the whole chain;
it asks a node for the parts that matter to it. Whoever runs that node —
nightfallcoin.org by default, you with My Node — is trusted for chain data.

| Your node **can** | Your node **cannot** |
|---|---|
| See the IP address of every visitor | See recovery words, keys or balances — keys stay in the browser |
| See each transaction a visitor sends (it relays them) | Spend anyone's coins or change a payment's amount or recipient |
| Refuse to serve, or go offline | Make the wallet accept a page that does not continue its saved position and anchor hash |
| Hide an incoming payment from the wallet, or show one that is not on the real chain, by lying consistently | Make the wallet accept a different network: every answer is checked against the mainnet genesis |

What the wallet checks: the genesis hash; that every page begins exactly at
the wallet's last scanned block and carries that block's hash; that headers
are continuous and hash-linked; and that the chain did not change between the
reads that make up a page (a page is read as headers → outputs → headers again
and refused if the two header reads differ). What it does **not** check is
proof of work. That is the price of being light, it is the same with or
without My Node, and it is why the node you choose matters. Running your own
means the node you trust is yours.

## How it fits together

```
browser ── https://wallet.example.org ──► Caddy ──► static wallet files
                         POST /wallet-api ──► 127.0.0.1:17888  nightfalld --mobile-listen
```

One origin. The wallet's Content-Security-Policy allows `connect-src 'self'`
only, so the page can talk to the host it was loaded from and nothing else.
The reverse proxy makes your node part of that host. An "enter a node URL"
box would have needed the policy opened to arbitrary hosts, which is exactly
what the policy exists to prevent; it was not built.

nightfallcoin.org answers the wallet's `wallet_scan` request in its Worker.
A plain node has no `wallet_scan`. The wallet notices the refusal on the
first page and from then on performs the same bracketed reads itself
(`nodeClient` / `anchoredScan` in `session.js`), with the Worker's checks
line for line. A node that answers "rate limited" is waited out with a
doubling delay instead of failing the sync.

## Setting it up

1. **An archive node with the light API on loopback.** A pruned node cannot
   serve it (the node refuses to start that way).

   ```
   nightfall-nodekit service --kind systemd --binary /usr/local/bin/nightfalld \
     --datadir /var/lib/nightfall --user nightfall --public \
     --light-port 17888 --out ./nightfall-mainnet.service
   ```

   The unit binds the light API to `127.0.0.1:17888` only. Install it as the
   first lines of the file describe, then check it with
   `nightfall-nodekit doctor` and `nightfall-nodekit status`.

2. **The wallet's static files** in a directory, e.g. `/srv/nightfall-wallet`
   (`index.html`, `app.js`, `session.js`, the `pkg/` WebAssembly build and the
   rest of the wallet origin).

3. **A Caddyfile to review.**

   ```
   nightfall-nodekit selfhost --domain wallet.example.org \
     --wallet-dir /srv/nightfall-wallet --out ./Caddyfile
   ```

   It serves the files with the same response headers as the official wallet
   origin, forwards only `POST /wallet-api` to the node, caps request bodies
   at the node's own 512 KB, and gets a TLS certificate automatically. The
   wallet needs HTTPS: browsers only persist storage and expose WebCrypto in a
   secure context. Install it yourself (`sudo cp Caddyfile /etc/caddy/`).

4. Open `https://wallet.example.org`. Restoring from recovery words works as
   on the official site; the wallet itself is identical. Storage is per
   origin, so a wallet saved on wallet.nightfallcoin.org does not appear on
   your domain — restore it there from its words or an exported backup.

## Explicit proxy trust in local 1.1.0

The default rate limit is based on the TCP peer. To separate visitors behind a
local **HTTP** proxy, start the node with `NIGHTFALL_TRUST_LOOPBACK_PROXY=1` only
when that proxy overwrites or appends the actual client address to
X-Forwarded-For. Remote sockets never gain this trust. Do not enable it behind
socat or another raw TCP forwarder: it forwards an attacker's headers unchanged.
Without this opt-in all visitors forwarded over loopback share one budget.

Light-API execution now uses eight fixed workers and at most sixteen queued
sockets. Reading headers/body has a twenty-second budget including queue time;
response writes also time out. Excess sockets close immediately. Rate-limit
entries expire after sixty seconds and the table is capped at 4096 clients.
These bounds limit process resources; a production proxy still needs its own
connection and request limits.

The older test evidence below predates the explicit proxy opt-in. New local
checks must supply the opt-in for the trusted HTTP proxy scenario.

## Rate limits behind a proxy

The node allows 120 light-API requests per minute per client. Behind a proxy
on the same machine every request arrives from 127.0.0.1, which would put all
of your visitors in one bucket — a single fresh scan (three requests per
512-block page) would lock out everyone else for a minute. So when, and only
when, a request comes from loopback, the node charges it to the last address
in `X-Forwarded-For`, which the proxy itself appended. From anywhere else the
header is ignored, so nobody can use it to pick someone else's budget or get
a fresh one. Caddy replaces any `X-Forwarded-For` a visitor sends with their
real address unless you configure it to trust upstream proxies.

## Evidence

Tested locally on 30 September 2026:

- `scripts/check-wallet-my-node.mjs` — the real scan loop through the fallback
  against a simulated plain node: full sync to the tip, reorg between the
  reads refused with nothing saved, header gap refused, pruned history
  refused, out-of-range requests never sent, rate-limit backoff and its limit,
  no silent mode switch on other errors. Added to the website deploy gate.
- `mobile.rs` unit test `only_a_local_proxy_may_say_who_the_client_is`.
- Node Kit tests for the light-API service flag and the Caddyfile
  (policy identical to the Worker's, single loopback upstream, POST only,
  hostname/path refusal).
- `scripts/e2e-my-node-devnet.mjs` — against a real local **devnet** node
  (40 blocks, light API on 127.0.0.1:27888): `wallet_scan` refused by the
  plain node, fallback scanned 0..39 in 7 hash-linked pages ending on the
  node's tip hash; with `X-Forwarded-For` from loopback, visitor A was refused
  on request 121, visitor B was still served, and an address a visitor
  prepended did not change whose budget was charged. This ran the modified
  `nightfalld` itself, not a simulation. Mainnet was not touched.

**Not evidenced:** the generated Caddyfile has not been loaded by a Caddy
binary (none is installed here, and installing software was out of scope for
the night); nobody outside the project has self-hosted the wallet; the web
wallet's static source is not in the public repository, so a self-hoster
currently has to copy the files from the served site. Publishing it is a
decision for the maintainer, not something done tonight.

## 11 October local Caddy verification

The generated configuration was validated by real Caddy 2.11.7 (official release archive, SHA512 matched the official checksum list). A loopback HTTP run then checked static files/CSP, POST-only API proxy, no-store and oversized-body rejection against the public synthetic fixture. No public TLS domain or live server was configured. The checked executable remains a temporary tool under target/local-tools; it was not installed system-wide.
