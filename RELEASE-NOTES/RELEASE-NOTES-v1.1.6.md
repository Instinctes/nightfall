# NIGHTFALLCOIN 1.1.6 — a new look for Core

1.1.6 gives Core a new design with motion throughout. The sidebar keeps its
place and shape. Consensus rules, supply, emission, block reward, block time,
proof of work, the wallet file and the data folder are unchanged, and nodes on
1.1.1 to 1.1.5 stay fully compatible.

## The balance

- **A living balance card.** A light pastel sky with a moving aurora and
  hashes raining softly behind it. On first open the card starts lit and the
  lights go out in a soft sweep while your balance counts up.
- **Rolling numbers.** The balance rolls like a counter whenever it changes.
- **Send and Receive** sit directly on the card.

## Moving around

- **Status island.** A pill at the top of the window shows the sync state.
  For a few seconds it widens with the news when a block is found, a payment
  arrives or a payment is submitted.
- **Page transitions, springs and a living sidebar.**
  - Pages fade and rise into place.
  - The selection glides like liquid, and each icon has its own small motion.
  - Mining shows dancing bars in the sidebar while it runs.
- **Quick search.** ⌘K (Ctrl+K) opens a palette to go to any page, start or
  stop mining, lock the wallet or reduce motion. ⌘1 to ⌘7 open the pages.
- **iOS-style switches, sliding segmented controls, softer toasts.**

## Pages

- **Send:** the last step is **Slide to send**. It does exactly what "Send
  now" did, and Enter still sends when the knob has keyboard focus.
- **Mining:**
  - a large power button;
  - while mining, nonces spiral into it at a pace set by your hashrate;
  - a shockwave rolls out when you find a block;
  - a strip of the latest blocks slides along as the chain grows;
  - a smooth hashrate chart and a new slider for CPU threads.
- **Network:** a turning globe with the real continents and one light per
  peer, joined by arcs that carry pulses; drag it to spin. The continents are
  built into the app. The positions of the peers are illustrative: Core never
  looks up where a peer is.
- **Receive:** a cleaner QR code; the modules stay square and dark on white
  so it scans as before.

## Fixes

- **MAX on Send** wrote the amount with a thousands separator ("2,747.999…"),
  which the amount field then rejected. From 1,000 NIGHT up, MAX did not work.
  It now fills in an amount the field accepts.

## Comfort

- **Reduce motion** (Settings → Window) turns every animation off; everything
  still works the same.

## Upgrading

Close Core completely before replacing the app or binary. Keep your 24 words
and an encrypted wallet backup, and never delete your data folder to update.
Nothing has to be migrated.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are
built by GitHub Actions with `--locked`, gated by platform tests, and published
with SHA256 manifests and GitHub build provenance. Verify with
`scripts/check_release_provenance.py` or `gh attestation verify <file> --repo
Instinctes/nightfall`. Provenance is not Apple notarisation, Windows
Authenticode signing or an independent security audit.
