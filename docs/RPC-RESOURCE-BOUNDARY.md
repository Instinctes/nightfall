# Paradigm Creator: resource-sovereign local RPC (experimental)

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
| Complete inbound JSON frame | 10 seconds, 4 MiB | Absolute deadline, not reset by trickling bytes |
| Any RPC session lifetime | 10 minutes | Valid pipelining cannot monopolize a slot forever |
| Streaming session lifetime | 10 minutes | Subscribers must reconnect on normal expiration |
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
- A `scan_subscribe` caller must reconnect and resume from its last received
  height after ten minutes. Some older clients may require a reconnect fix.
- Tests cover slot exhaustion/reclaim, two pipelined JSON frames, truncated
  frames and size overflow. A real multi-client soak test is still required
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

Security review and a real-device reconnect test are required before merging.
Do not run fault-injection against live mainnet RPC.
