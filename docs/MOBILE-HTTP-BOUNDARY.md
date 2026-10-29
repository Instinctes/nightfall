# Mobile Light API: resource and proxy boundary

This document describes the current mainline
implementation after the PR #70 forward-port.

It is not an independent security audit.

## Resource limits

- Only allow-listed Light API RPC methods
  are accessible.

- Full RPC mining and full-block methods
  are intentionally excluded.

- 8 fixed workers process a bounded queue
  of 16 connections.

- Excess sockets are dropped.

- Queue time counts toward the 20-second
  request budget.

- HTTP headers are limited to 16 KiB.

- Request bodies are limited to 512 KiB.

- Duplicate Content-Length and
  Transfer-Encoding are rejected.

- Rate limits are 120 requests per minute.

- The source table is bounded to 4096 entries.

## TCP forwarders

A plain TCP forwarder can present all clients
as 127.0.0.1.

Without explicit proxy trust, these clients
share one 120-per-minute source bucket.

## Trusted local HTTP reverse proxy

NIGHTFALL_TRUST_LOOPBACK_PROXY=1

This setting is only for a locally controlled
HTTP reverse proxy that correctly overwrites
or appends trusted X-Forwarded-For values.

It must not be enabled without such a proxy.

Requests from non-loopback socket peers
ignore forwarded-client headers.

No PROXY protocol v1/v2 support is claimed.

## Regression coverage

Tests check:

1. Shared quotas behind raw TCP forwarding.
2. Separate quotas for trusted local clients.
3. No quota bypass from untrusted headers.

## Compatibility

The former PR #70 handler, which could
spawn up to 64 handler threads, has not
been restored over the newer mainline
fixed-worker implementation.

This avoids weakening the current
resource protection.

Real-client load testing, protocol review,
security review and maintainer approval
remain necessary before release.
