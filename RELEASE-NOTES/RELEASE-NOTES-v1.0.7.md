# NIGHTFALLCOIN 1.0.7 — a seed that keeps its memory

1.0.7 fixes a memory problem that made the archive seed — the node behind the
web wallet and the website's live figures — slow, and finally killed it. Same
chain, genesis, protocol v8 and wire v6; no consensus rule changed. It is also
the first release whose every file is built and signed by GitHub Actions.

## Forks no longer copy the whole chain

When a peer has a different tip, a node looks at the peer's branch to decide
whether to switch. In 1.0.6 both ways in — pulling from a peer, and a peer
pushing a competing block — first copied our own history and only then asked
the cheap questions: is the fork deeper than 500 blocks, is another check
already running, is the peer's branch heavier at all? On a pruned node that
history is 500 blocks. On an archive node it is every block since genesis.

On the archive seed, peers on old dead forks and ordinary competing blocks made
it do that copy again and again, several at once and partly while holding the
node's state lock: memory jumped from 0.7 GB to 7.6 GB within seconds, the light
API took up to 20 seconds to answer (the website showed "Seed did not answer"),
and on 16 October the kernel killed the node for running out of memory.

1.0.7 asks the cheap questions first and copies only for a branch it is
actually going to verify — one at a time:

- **Pull path:** depth limit and the one-at-a-time guard come first; the peer's
  suffix is fetched and its declared work compared with the blocks of ours it
  would replace; only a heavier branch makes the node copy its history.
- **Push path:** the competing run is weighed against our blocks after the fork
  point before anything is copied, and the fork point is searched only within
  the 500-block reorg window instead of hashing every block since genesis under
  the lock.
- A heavier branch is still verified and adopted exactly as before.

Tests: a two-node test with an archive node and a peer ahead on a branch that
diverges at genesis — refused, with no copy of history. Run against the 1.0.6
code it fails (the history is copied before the depth check); with 1.0.7 it
passes. The new checks are tested to agree with the copying ones on archive and
pruned chains. All earlier reorg tests pass, including the #40 two-node test.

Who should update: **anyone running an archive node** (not pruned), and seed
operators. Desktop wallets benefit too, but were rarely affected.

## Every wallet built and signed by GitHub Actions

Windows, Linux and both macOS DMGs (Apple Silicon and Intel) are now built on
GitHub runners from this tag, and every file carries a signed build provenance
(Sigstore). Check a download:

```bash
gh attestation verify NIGHTFALLCOIN-Core-1.0.7-macOS-arm64.dmg --repo Instinctes/nightfall
```

or `python3 scripts/check_release_provenance.py --tag v1.0.7 --platform macos
--directory <folder>`. This proves which workflow built a file from which tag.
It is not an Apple or Microsoft code signature — macOS and Windows still warn on
first launch. Phase 1 by @nightfall-wizard (#46); see
[docs/RELEASE-PROVENANCE.md](../docs/RELEASE-PROVENANCE.md).

## Also in this release

- **Privacy lockdown** (#47, @nightfall-wizard): with
  `NIGHTFALL_PRIVACY_LOCKDOWN=1` a node never connects directly when Tor is
  down, skips the direct HTTPS peer directory, and Core does not contact
  nightfallcoin.org. Off by default; see [docs/PRIVACY.md](../docs/PRIVACY.md).
- **Tests** (#44, #45, @nightfall-wizard): adversarial payment-request vectors
  and block-response boundaries for the 1.0.6 P2P fix.
- **Operator evidence script** (#48, @nightfall-wizard): `scripts/node-evidence.py`
  — read-only, redacted status receipts from your own node, to compare with
  another operator's.
- Security contact is now instinctes@hotmail.com.
- README: Argon2id rules out ASICs, not GPUs — graphics cards are faster than
  CPUs, and third-party GPU miners exist.

## Upgrade

No chain reset; wallet format unchanged. Quit Core fully before replacing the
application. Node operators: replace `nightfalld` and restart.

Downloads: macOS arm64/Intel DMGs, Windows and Linux Core, node and CLI wallet,
platform SHA256 lists and build attestations. Not Developer-ID signed or
notarized; Windows binaries have no publisher certificate.
