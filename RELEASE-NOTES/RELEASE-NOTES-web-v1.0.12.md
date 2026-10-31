# NIGHTFALL Web Wallet 1.0.12

A separately built browser release based on the stable 1.0.12 code. The Core
v1.0.12 tag and its official desktop/node files are unchanged.

- Recovery phrases are bounded before normalization; malformed-word/checksum
  diagnostics do not reflect input (#71, thank you @nightfall-wizard).
- Legacy raw-state WASM exports remain removed; BrowserVault/request APIs remain
  available (#37, thank you @nightfall-wizard).
- The browser's encrypted-envelope budget is 64 MiB, including overhead. Native
  Core keeps its larger 256 MiB plaintext budget. Larger backups belong in Core.
- Compiled version and browser backup-budget exports let the host check the
  actual module before opening browser storage.
- GitHub builds locked dependencies with Rust 1.98.0 / wasm-bindgen 0.2.100,
  exercises real WASM and native/browser backup interoperability, and publishes
  checksum manifests plus signed build provenance for the exact deployment bytes.

The site deploys the module under its own versioned path and preserves the old
module for already-open tabs. Domain, IndexedDB identity, encrypted format,
recovery seed derivation and transaction confirmation safeguards are unchanged.
Existing wallets do not require recreation. Private 1.1.0 work and withdrawn
atomic swaps are outside this release. This remains a light wallet trusting
node-provided chain data, not an independently audited or PoW-verifying client.

The matching website rollout has its own storage, upgrade and UI acceptance
checks. Core stays the latest desktop release; this browser release does not
replace it as GitHub's latest release.
