# NIGHTFALLCOIN 1.0.12 — bounded network responses and safer wallet scans

1.0.12 is a maintenance release on the stable 1.0.x line. It improves network
resource handling, CLI wallet scan validation and recovery-input handling.
Same chain and genesis, protocol v8 and wire v6. Existing consensus size limits,
PoW, emission, rewards, seed derivation and wallet storage formats are unchanged.

## Included

- **Early oversized-block rejection — #12.** Check existing aggregate input,
  output and kernel limits before expensive proof-of-work validation. Invalid
  oversized blocks are rejected sooner; the limits themselves are unchanged.
- **Bounded P2P block responses — #15.** Serialize within the wire-size budget
  before writing, fit block responses to that budget, and continue requesting
  subsequent pages. Oversized frames cannot leave a partial write; an individual
  block that cannot fit produces an explicit failure. Response counts and
  consecutive heights are validated and buffered interleaved messages preserved.
- **CLI wallet RPC scan validation — #69.** Refuse gaps, duplicate/reordered
  heights, oversized pages and height overflow before wallet reconciliation.
  Continue scanning valid one-block pages instead of ending early.
- **Recovery-phrase boundary — #71.** Refuse input above 1024 UTF-8 bytes before
  normalization and use generic malformed-word/checksum diagnostics. Valid
  24-word English BIP-39 phrases recover the same seed and address as before.
- **Strict SOCKS5 replies — #67.** Reject replies with a nonzero reserved byte.
- **Smaller WASM source API — #37.** Remove legacy raw-state wallet exports and
  retain the BrowserVault/request interfaces. This source change does not rebuild
  or change the separately deployed 1.0.7 browser wallet.

## Validation

The release source includes additional real TCP regressions for paginated P2P
responses with interleaved frames, zero partial writes on oversized frames,
valid one-block RPC pages and wallet-state preservation on malformed RPC pages.
Recovery tests cover input bounds, redacted errors and 128 deterministic test
seeds. All tests use synthetic fixtures rather than real wallet data.

Official release publication requires the GitHub Actions workspace tests and
builds for Windows x64, Linux x64 and macOS, plus matching version checks, SHA256
manifests and signed GitHub build provenance. Both macOS arm64 and Intel DMGs
are built by that workflow. No laptop-built release binary is substituted.

## Thanks

Thank you [@nightfall-wizard](https://github.com/nightfall-wizard) for
[#12](https://github.com/Instinctes/nightfall/pull/12),
[#15](https://github.com/Instinctes/nightfall/pull/15),
[#69](https://github.com/Instinctes/nightfall/pull/69),
[#71](https://github.com/Instinctes/nightfall/pull/71),
[#67](https://github.com/Instinctes/nightfall/pull/67) and
[#37](https://github.com/Instinctes/nightfall/pull/37), including the regression
tests and follow-up review handoffs.

## Upgrading and scope

Close Core completely before replacing the app or binary. Preserve your 24 words
and an encrypted backup; never delete the data directory to update. No chain
reset, wallet migration or seed service upgrade is required for compatibility.
The separately hosted web wallet remains on its tested 1.0.7 channel.

Build provenance proves origin from the tagged workflow. It is not an independent
security audit, Apple Developer ID/notarisation or Windows Authenticode signing.
Atomic swaps remain withdrawn. The private 1.1.0 roadmap work is not included.
