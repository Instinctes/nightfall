# Core and Web Wallet 1.0.5 validation — 21 September 2026

Source: `9591ac0a3a58f734883f8b248b7b05aac88ee8f0`, tag `v1.0.5`.

## Local checks

- Rust 1.98.0: `cargo +1.98.0 test --offline --locked --workspace` passed.
  446 regular tests plus two isolated subprocess lifecycle runs passed.
  Three default skips include the explicitly invoked subprocess helpers and a
  timing benchmark. No real wallet or mainnet payment was used.
- `cargo +1.98.0 clippy --offline --locked --workspace --all-targets -- -D warnings`
  and formatting checks passed.
- Core lifecycle coverage includes detached candidate conflicts, automatic
  reconciliation, retained reservations, quarantined retries, refusal of old
  swap-lock sends and preflight refusal of unavailable rescan history.
- Air retries retain the same signed answer after restart, reject changed nonce
  terms and keep the journal through rescan. Failed relays remain retryable.
- Rewards are deduplicated across repeated frames/reorg/rescan/unlock, catch-up
  is quiet, sound preference persists, failed preference writes report failure,
  and the generated WAV is bounded valid PCM without clipping or edge clicks.
- The opt-in native renderer captured startup, Mining and a reward preview in
  an isolated Devnet directory. Visual inspection found one reward card and a
  separate readable sound toggle. Other wallet pages were also captured.
- Real-WASM web checks: 15 passed, including persistence-before-broadcast,
  save/lock conflicts, imported-payment quarantine, automatic reorg recovery,
  unavailable archive preflight and bounded repeated-branch recovery.
- Browser integration: changing the isolated fixture chain under an existing
  saved payment recovered to the new tip automatically, without a warning or
  confirmation dialog; the old payment stayed withheld and reserved.
- Migration checks: 16; scan-proxy checks: 15; website security checks: 49.
  Tests use public synthetic keys. The legacy wallet files remain unchanged.

## Packaging and external gates

Both macOS DMGs were rebuilt after the final reward changes, verified by
`hdiutil`, checked for minimum OS 11.0 (arm64) / 10.15 (Intel), ad-hoc signed,
checksummed and uploaded to a draft release. These are not notarized builds.

GitHub CI: https://github.com/Instinctes/nightfall/actions/runs/35646590582

GitHub release jobs: https://github.com/Instinctes/nightfall/actions/runs/35646670905

Both linked runs completed successfully. CI built all three desktop platforms,
ran formatting/Clippy/workspace tests and the exploit/reorg regressions. Release
jobs ran the workspace tests on Windows and Linux, checked the binary version
against the tag and produced matching platform checksum lists. All eight local
release binaries/installers match their SHA256 lists; Windows lists are LF-only.

GitHub v1.0.5 was published on 22 September 2026 (03:34 UTC) with eight binaries /
installers and three checksum lists. The website and Web Wallet 1.0.5 were deployed on 22 September after all 14
site gates passed. Cloudflare Worker version:
`e64422f7-6a62-43e4-957f-679868750ed9`.

Nineteen live assets matched the tested local files or published SHA256 lists:
all eight binaries/installers, all three checksum lists, release metadata,
wallet HTML/JS/session/CSS/WASM, the iPhone icon and standalone manifest.
The home page matched local HTML, all eight public pages answered, the wallet
origin returned 200/no-store, preview redirected to `/`, the legacy exporter
returned 200, and the apex refused `/wallet-origin/` with 404.

No existing Core process or real wallet directory was stopped or replaced.

## Boundaries

This is targeted regression and integration validation, not an independent
security audit or proof that all possible wallet defects are absent. Windows
and Linux hardware audio playback was not auditioned. Linux needs `pw-play`,
`paplay` or `aplay`. Missing archive data and deep stuck node forks remain
subject to node safety rules; the UI does not automatically delete chain files.
The web wallet validates continuity against its configured node and does not
independently verify proof of work. Snapshots containing the new Air signing
journal need version 1.0.5 or later.
