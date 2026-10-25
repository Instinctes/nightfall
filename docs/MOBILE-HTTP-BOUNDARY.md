# Mobile HTTP Boundary — experimental security hardening

Security hardening for Nightfall's public light-client API.

No change to V8 consensus, emission, P2P wire format,
wallet keys or RPC method allowlist.

## Limits

- Maximum 64 concurrent handlers
- Maximum 4096 unique-IP rate buckets
- Existing per-IP 120/minute rate policy retained
- Maximum 16 KiB HTTP header
- Maximum 512 KiB HTTP body
- 20-second absolute request-read deadline
- Duplicate Content-Length rejected
- Transfer-Encoding rejected
- One request processed per connection

## Testing

Offline Rust format, focused tests, cargo check and Clippy.
Tests cover fragmented requests, malformed framing,
oversized bodies, admission limits and source-bucket limits.

## Limitations

These controls do not prevent distributed denial of service,
network censorship, malicious chain data, wallet compromise,
or privacy leaks outside this listener.

TLS requires a trusted reverse proxy.
Security review and production soak tests are required.
