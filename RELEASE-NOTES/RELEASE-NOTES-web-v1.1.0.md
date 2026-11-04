# NIGHTFALL Web Wallet 1.1.0

The browser wallet built from the 1.1.0 code. **Required before block
500,000**: from that height the network only accepts transactions in the 1.1.0
format, and this module builds them. Payments sent with it before the
activation height stay valid after it.

- Transactions carry the 1.1.0 binding fields (input keys, kernel stealth
  excess) and, once the next block is at or above 500,000, a kernel offset.
  See `docs/SPEC.md` §1.6.
- Failed payments stay in history, marked as not completed.
- GitHub builds locked dependencies with Rust 1.98.0 / wasm-bindgen 0.2.100,
  exercises real WASM and native/browser backup interoperability, and publishes
  checksum manifests plus signed build provenance for the exact deployment bytes.

The site deploys the module under its own versioned path and preserves the old
module for already-open tabs. Domain, IndexedDB identity, encrypted format and
recovery seed derivation are unchanged; existing wallets do not need to be
recreated. This remains a light wallet trusting node-provided chain data, not
an independently audited or PoW-verifying client.
