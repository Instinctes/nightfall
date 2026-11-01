# Light API ingress: bounded work and read-only framing

This non-consensus patch targets the v1.1.0 fixed-worker/16-queue Light API.

It intentionally does not bring back per-connection detached threads from
older PRs.

Workers, the 4096-bucket cap, quotas, proxy settings and the 20-second
per-request budget remain unchanged.

## Changes

- Existing source requests no longer scan the entire rate-limit map.
- Unknown sources at full capacity trigger bounded expiry reclamation.
- GET and OPTIONS reject ambiguous request framing.
- Duplicate Content-Length and unsupported transfer encodings are rejected.
- Nonzero Content-Length is rejected on read-only requests.
- Exact 60-second quota rollover is covered by regression tests.
- The existing public RPC allowlist is unchanged.

## Proxy limitations

A plain TCP forwarder does not preserve the actual client IP.

This change does not implement PROXY protocol and does not solve shared
rate-limit quotas behind an ordinary TCP forwarder.

Operators must preserve the existing trusted-proxy security boundary.

## Validation

Run:

cargo test --locked -p nightfall-node --lib mobile::tests::

cargo build --locked -p nightfall-node --bin nightfalld

python3 scripts/test-light-api-boundary.py

The real HTTP test starts a disposable loopback Devnet node.

No mainnet access, wallet modification or blockchain reset is required.

## Remaining work

Independent security review, production proxy compatibility tests,
multi-client load testing and resource measurements are still required.

This patch makes no claim of DDoS immunity or independent consensus audit.
