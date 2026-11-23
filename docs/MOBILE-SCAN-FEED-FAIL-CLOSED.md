# Mobile `scan_feed`: fail closed on malformed pages

## Risk

The pre-patch UniFFI client silently filtered malformed outputs and spent
commitments while it could still persist the page's `scanned_to` height.
An honest-but-buggy server could thus leave a gap that the next incremental
scan would not revisit. A hostile node can omit chain data regardless; this
patch **does not authenticate the remote chain**.

## Change

Before calling `Wallet::ingest_scan_page`, require the page to state the exact
requested start and a contiguous bounded block interval; check the complete
output/spend arrays, field types, 32-byte hex values, the v8 fixed-size sealed
payload, view-tag range, heights, and optional tip and archive-horizon claims.
Reject the whole page on any malformed record, instead of dropping that
record and advancing. Empty pages must contain no records and no progress.

The page parser is separate from key material and has offline regression
tests with malformed and adversarial JSON. No network, real wallet, keys or
payments are touched by tests. Normal pages follow the unchanged ingestion
logic. A valid-looking response is **not** independent verification, and a
malicious node can still withhold a valid response or lie consistently.

## Verification

cargo fmt --all --check
cargo test --locked -p nightfall-mobile --lib mobile_scan_feed_tests
cargo clippy --locked -p nightfall-mobile --all-targets -- -D warnings

No new CI workflow, protocol change, Mainnet activation or data migration.
Independent mobile-device integration, genuine operator evidence and reviewer
approval remain necessary before shipping in a wallet release.
