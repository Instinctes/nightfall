# Recovery-phrase input boundary

Status: non-consensus security hardening.

## Threat model

Core BIP-39 parsing is exposed through more than one host.
The cryptographic entry point must enforce its own input bound
even if a wallet UI already validates input.

A 1024-byte raw UTF-8 limit is enforced before splitting,
normalization, case folding, or intermediate buffer allocation.

Malformed word and checksum errors use a generic diagnostic.
Third-party parser diagnostics are not forwarded to callers.

This reduces unnecessary allocation exposure and establishes
an error-redaction invariant for recovery handling.

## Preserved invariants

- Exactly 24 BIP-39 English recovery words.
- Identical seed-to-address mapping.
- Identical checksum validation.
- No changes to transaction signing.
- No changes to protocol v8 or consensus.
- No changes to issuance or supply rules.
- No changes to wallet storage format.

## Validation

Run:

cargo test --locked -p nightfall-crypto --tests

cargo clippy --locked -p nightfall-crypto   --all-targets -- -D warnings

cargo fmt --all --check

Then require the normal full-workspace GitHub Actions suite.

The implementation is not an independent cryptographic audit.
It cannot protect recovery words on a compromised device.
