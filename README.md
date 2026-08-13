# nightfallcoin.org — website

Static single page, deployed as a Cloudflare Worker.

**Live:** https://nightfallcoin.org

No build step, no framework, and no third-party requests — a page about
privacy has no business loading fonts or analytics from someone else's server.

```
website/
├── wrangler.toml       Worker + static asset config
├── src/index.js        the Worker: security headers, nothing else
└── public/             everything served
    ├── index.html
    ├── 404.html
    ├── css/style.css   palette mirrors the Core Wallet
    ├── js/main.js      scroll reveals, counters, canvas mesh gradient
    ├── assets/         logo and favicon (SVG)
    └── downloads/      wallet binaries
```

## Why a Worker and not plain Pages

The site serves wallet binaries. If someone could inject a script into the
page, they could swap a download link and drain everyone who trusted the
domain. The Worker exists to put a strict `script-src 'self'` policy — plus
HSTS, `nosniff`, `frame-ancestors 'none'` and a locked-down permissions
policy — on every response. That costs nothing here, because the site
deliberately loads no external code at all.

Two settings in `wrangler.toml` are load-bearing:

- **`run_worker_first = true`** — without it Cloudflare serves static files
  straight from the edge and the Worker never runs, silently dropping every
  security header. The headers are the entire reason this is a Worker.
- **`not_found_handling = "404-page"`** — an SPA fallback would return HTTP 200
  with HTML for a missing path, so a broken download link would quietly save a
  web page as a `.dmg` instead of failing loudly.

## Develop

```bash
cd website
wrangler dev          # http://localhost:8787, headers included
```

Or serve the static files without the Worker:

```bash
cd website/public && python3 -m http.server 8080
```

## Deploy

```bash
cd website
wrangler deploy
```

## Releasing a new build

`public/downloads/` is gitignored — the binaries live in the repo's releases,
not its history.

```bash
./scripts/build-macos-dmgs.sh
cargo build --release --target x86_64-pc-windows-gnu -p nightfall-core

cp wallets/*.dmg website/public/downloads/
cp wallets/windows-x64/nightfall-core.exe website/public/downloads/

cd website && wrangler deploy
```

Then verify what the edge is actually serving matches what you built — the
whole point of publishing checksums is undermined if the CDN has something
else:

```bash
U=https://nightfallcoin.org
curl -s $U/downloads/NIGHTFALLCOIN-Core-macOS-arm64.dmg | shasum -a 256
shasum -a 256 wallets/NIGHTFALLCOIN-Core-macOS-arm64.dmg
```

## Domains

`nightfallcoin.org` and `www.nightfallcoin.org` are declared as custom domains
in `wrangler.toml`, so `wrangler deploy` creates the DNS records and requests
the certificate itself — there is nothing to click in the dashboard.

The Worker answers on the apex and redirects `www` to it with a 301. One
canonical spelling matters more here than on an ordinary site: visitors are
asked to compare download checksums, and a second legitimate hostname is a
second thing a phishing domain can imitate.

Attaching a custom domain disables the `*.workers.dev` hostname unless
`workers_dev = true` is set explicitly. That is intended — the old URL should
not keep serving wallet binaries alongside the real one.

## Editing content

Everything worth changing is plain text in `public/index.html`. Two things to
watch:

- The **limitations section is not decoration.** If a limitation gets fixed,
  move it out; if a new one appears, put it in. A privacy project that quietly
  drops its caveats has stopped being honest.
- The **seed node** is `seed.nightfallcoin.org:17891`, compiled into builds from
  `main` via `NetworkId::seed_nodes()`. If that address ever changes, it has to
  change in both places, and the shipped binaries only pick it up on the next
  release — which is the argument for keeping it a DNS name.
