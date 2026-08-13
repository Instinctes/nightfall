**Protocol v6 — Nightproof.** The chain was reset and restarted from a new
genesis. Nothing from v5 carries over.

```
genesis   b69d9c81892266a7b89b2e759f9cfd4d9344230b084545d4f92d648ab9eb11a1
```

---

## Why there was a reset

Two v5 chains grew apart on separate networks — one machine on an office LAN,
the other alone on a phone hotspot — and could no longer reconcile. Both were
internally valid, both passed their own supply proof, and neither looked wrong
until someone compared them by hand. Rather than pick a winner, both were
abandoned.

A reset is also the only cheap moment to change the output format, so that
happened at the same time.

## What is new

**View tags.** Every output now carries one byte derived from the sender's
shared secret. A wallet compares it before the second scalar multiplication, so
outputs belonging to strangers — which is nearly all of them — are discarded
after one unavoidable operation instead of a full key derivation. Measured, not
estimated:

```
per foreign output    before 61,590 ns    after 30,321 ns    2.03x
```

Reproduce it yourself: `cargo run --release -p nightfall-crypto --example scanbench`

The tag is covered by the sender's signature. It has to be — a relay that could
flip that byte would make the output invisible to its recipient, and funds
nobody can find are funds destroyed.

This matters because it makes a phone wallet realistic. Monero shipped the same
construction in 2022 and needed a hard fork; doing it during a reset cost
nothing.

**BIP-39 recovery phrases.** Wallets now show 24 words. The 32-byte seed is
used as BIP-39 *entropy* rather than run through PBKDF2, so the round trip is
exact and wallets that already exist can produce words for the seed they
already have. A backup scheme that only works for wallets created after it
shipped is not a backup scheme.

These words are **not** interchangeable with a Bitcoin or Ethereum wallet. The
encoding is standard; the key derivation is not.

**Birth height.** A wallet cannot own an output that predates its keys, so a
wallet created at the tip has nothing to scan. Without this a fresh install
walks the entire chain to find nothing.

**A light-client feed.** `scan_feed` serves the fields a wallet actually reads
and omits the Bulletproofs, which are ~672 bytes per output and irrelevant to
finding your own coins. Measured on devnet: 8× less data than fetching blocks.

**Honest sync status.** The dashboard used to show a green "In sync" beside a
warning that read "not connected to anyone". It was reporting whether scanning
had finished, which says nothing about agreement with a network when there is
no network. With no peers it now reads `No peers · local block N`.

**Fork choice compares work before length.** A longer but lighter chain was
being rejected as `ReorgTooDeep` — reading as "this looks like an attack" when
the truth was "we compared the work and yours is lighter". The depth bound is a
denial-of-service limit, not a rule about which chain is correct, and it now
applies only to candidates that claim to be heavier.

---

## Downloads

| File | Platform |
|------|----------|
| `NIGHTFALLCOIN-Core-0.4.0-macOS-arm64.dmg` | Apple Silicon (M1–M4), macOS 12.5+ |
| `NIGHTFALLCOIN-Core-0.4.0-macOS-intel.dmg` | Intel Macs, macOS 12.5+ |
| `nightfall-core-0.4.0-windows-x64.exe` | Windows 10+ — the wallet |
| `nightfalld-0.4.0-windows-x64.exe` | Windows — headless node |
| `nightfall-wallet-0.4.0-windows-x64.exe` | Windows — command-line wallet |

Checksums are published as `SHA256SUMS-0.4.0.txt` (macOS) and
`SHA256SUMS-0.4.0-windows.txt`. Filenames carry the version deliberately: a URL
then refers to exactly one file, so no cache anywhere can hand you an old
binary under a new build's published checksum.

```bash
shasum -a 256 -c SHA256SUMS-0.4.0.txt                    # macOS / Linux
certutil -hashfile nightfall-core-0.4.0-windows-x64.exe SHA256   # Windows
```

**These builds are not code-signed.** That is the honest reason your operating
system will complain, and also why the checksums matter — they are the only way
to tell this build from one somebody substituted.

- **macOS:** right-click → **Open** → **Open**. Once only.
- **Windows:** SmartScreen warns about an unknown publisher. **More info → Run anyway**, if you trust the source.

The Windows binaries are built by
[a published workflow](https://github.com/Instinctes/nightfall/blob/main/.github/workflows/release.yml)
on GitHub's runners rather than on a laptop. That is not a reproducible build,
but it is a better claim than "trust the machine it came off".

If you would rather not take our word for any of it,
[build from source](https://github.com/Instinctes/nightfall#quick-start) — that
is the point.

---

## First run

1. Launch the wallet.
2. **Settings → Backup.** Write the 24 words on paper before you mine anything. Losing them loses the coins, permanently. There is no reset and no support desk.
3. **Connect to a peer** — see below.
4. **Start mining.**

Mined coins show as *unlocking* for 1,440 blocks (~6 h) before they can be
spent. That delay protects against chain reorganisations; the coins are already
yours.

### Connect before you mine

There is **no public seed node yet**, so the first connection is made by hand
in **Network → Add a peer** (`host:port`, default port 17891).

> Two miners who never meet build two separate chains from the same genesis.
> Both look perfectly valid locally. This has already happened on this network,
> and it is why the chain you are downloading is the second one. The wallet
> warns you while you are mining with zero peers — believe it.

Nodes only peer with a matching genesis hash.

---

## Honest status

**This is pre-launch software that has not been reviewed by anyone outside the
project.**

Protocol v4 shipped a balance proof that was a tautology: it computed a value
from public data, recomputed the same value, and compared it to itself. It
could not fail. Anyone could have minted unlimited NIGHT, and the recipient of
every payment was published in cleartext. The full analysis, including working
proof-of-concept exploits, is in
[`docs/AUDIT-2026-08-12.md`](https://github.com/Instinctes/nightfall/blob/main/docs/AUDIT-2026-08-12.md).
We publish our own failures because a privacy project that hides them is not
one you should use.

Still missing, stated plainly:

- **An independent audit.** The cryptography was written and reviewed by the same party. That is a conflict of interest, and 134 passing tests are not a substitute.
- **A public seed node.** Until one exists, joining requires being told an address by hand — which means, right now, that almost nobody can join.
- **Graph privacy.** Block-level aggregation hides which input paid which output within a block, but spent outputs stay visible. Cut-through would remove the signature that makes non-interactive payments safe, so the two are mutually exclusive.
- **Network-layer privacy.** No Dandelion++. The first node to relay your transaction is probably its origin.
- **A large network.** A young chain with little hashrate can be out-mined. That is true of every new proof-of-work network.

Do not put value on this that you cannot afford to lose entirely.

---

**Website:** https://nightfallcoin.org
**Source, spec and audit:** https://github.com/Instinctes/nightfall
