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

## Reverse proxies and shared quotas

The source IP is taken from the accepted TCP socket (`peer_addr`).
This is a **transport peer address**, not a verified wallet-user IP.

When a TCP forwarder sends public port 80 to loopback port 17888,
all downstream wallet users can appear as `127.0.0.1`. The existing
120-requests-per-minute source bucket is then **shared by every user**
behind that forwarder. Heavy traffic from one client can trigger HTTP
429 for unrelated wallets. This is an availability limitation, not a
wallet-specific restriction. The 64-active-handler limit is global
regardless of whether a forwarder is present; saturation returns 503.

No trusted-proxy mode is implemented here. The listener must **not**
interpret untrusted `X-Forwarded-For`, `Forwarded`, or arbitrary PROXY
headers as authenticated identities. Those values can be spoofed when
the direct listener is reachable. A loopback TCP forwarder does not
preserve the originating client IP by itself.

Operators using this patch behind a forwarder should enforce their own
per-client admission controls at a trusted, correctly configured edge,
keep the light listener inaccessible to untrusted direct connections,
and account for its additional shared 120/minute quota. Edge limits do
**not** remove the listener's shared quota: production suitability
requires an explicit load test with multiple independent clients.

A future opt-in proxy-identity feature needs a defined trusted-peer
boundary and separate adversarial review before it is enabled.

## Limitations

These controls do not prevent distributed denial of service,
network censorship, malicious chain data, wallet compromise,
or privacy leaks outside this listener.

TLS requires a trusted reverse proxy.
Security review and production soak tests are required.
