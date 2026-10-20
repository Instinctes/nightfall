# Bounded RPC resource admission (experimental)

## Threat and boundary

The pre-change `rpc.rs` spawned a new OS thread for every accepted TCP
connection. A peer could hold a connection without sending a complete line;
`scan_subscribe` could also occupy a worker indefinitely. An attacker with
local access (or remote access when an operator explicitly enables unauthenticated
public RPC) could exhaust threads and deny independent operators control of
their nodes. A line-size limit **alone does not bound wall-clock occupancy**.

The new admission protocol is intentionally **not consensus-active**:

| Limit | Bound | Effect |
| --- | --- | --- |
| Active RPC sessions | 32 | Reject excess sockets before spawning a worker |
| Long-lived scan subscriptions | 8 | Reserve at least 24 other session positions |
| Complete inbound JSON frame | 10 seconds from first byte, 4 MiB | Slow-drip frames cannot extend the deadline |
| Idle waiting for the next request | 60 seconds | Waiting does not consume the frame deadline |
| Non-streaming RPC session lifetime | 10 minutes | Valid pipelining cannot monopolize a slot forever |
| Streaming subscriptions | No fixed lifetime; at most 8 | Existing wallet follow clients remain connected |
| Write operation | 10 seconds | Stalled receivers do not hold a worker forever |

The existing JSON wire format, RPC names, genesis, validation, mining,
wallet data and monetary policy remain unchanged. Counter accounting uses
atomic compare-and-swap and RAII release even on error/unwind.

## Operational limits

- This bounds **accepted RPC resources**, not total network traffic or CPU
  consumed by a valid request. A busy miner must still be monitored.
- It is **not authentication or authorization**. Do not bind the unauthenticated
  full RPC on the public internet; never use `NF_ALLOW_PUBLIC_RPC` without
  a separate correctly configured authentication boundary.
- A rejected connection is closed without a JSON error to avoid blocking the
  accept loop. Consumers must retry with bounded exponential backoff.
- `scan_subscribe` stays open for existing `nightfall-wallet follow` clients.
  Its separate eight-slot limit and write timeout still bound server resources.
- Tests cover slot exhaustion/reclaim, delayed first bytes, idle timeouts,
  pipelined JSON frames, truncated frames and size overflow. A real multi-client and 10-minute follow soak test is still required
  before deployment. This module does not prove resistance to network-level
  DDoS or malicious chain data.

## Verification

From a fresh checkout:

```sh
cargo fmt --all --check
cargo test --locked -p nightfall-node rpc::tests -- --nocapture
cargo clippy --locked -p nightfall-node --all-targets -- -D warnings
git diff --check
```

Security review and a real-client 10-minute follow test are required before merging.
Do not run fault-injection against live mainnet RPC.
