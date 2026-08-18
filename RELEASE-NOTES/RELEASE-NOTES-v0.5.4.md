Same chain as v0.5.0 — protocol v7, genesis `c8614333…`. **Upgrade in place; no
reset, no reindex.**

This release fixes the fault behind every "the wallet froze", "it hangs
behind", and "we forked again" report since the network went live. It was one
bug, it was mine, and it had been there since the first reorg code was written.

## A chain rebuild ran while holding the node's global lock

When a peer's blocks fail to connect to our chain, the node pulls that peer's
chain and asks `maybe_reorg_to` whether it is heavier. That call rebuilds the
candidate from genesis and re-verifies every block — proof of work included.

It ran inside the mutex that guards all node state.

Measured on a live 1,737-block chain:

```
rebuild_from_blocks: 26.67 s for 1737 blocks (15.4 ms/block)
```

For those 26.7 seconds the node answered no RPC call, drew no interface frame,
served no peer, accepted no block and submitted none. The cost grows linearly
with the chain, so it gets worse every day. And because every peer thread that
failed to connect a block reached the same conclusion at the same moment, ten
peers meant ten sequential rebuilds — minutes of a completely frozen node.

**Why only miners saw it.** A node that only follows the chain never diverges:
incoming blocks connect, get appended, and the reorg path is never entered. The
moment a node mines a block while even slightly behind, its chain holds
something the network does not — so the next blocks that arrive do not connect,
the rebuild starts, the node freezes for half a minute, falls further behind,
and does it again. Self-sustaining, and invisible to a node with mining off.

Measured on the same machine, before and after:

| | v0.5.3 | v0.5.4 |
|---|---|---|
| RPC response while mining | no answer in 25,000 ms | 93–136 ms |
| Chain vs. network while mining | stalled, 34 blocks behind and growing | ±2 blocks, periodically ahead |

## The fix

`Chain::maybe_reorg_to` is now a wrapper over two halves:

- **`Chain::evaluate_reorg`** — the expensive part. A free function over
  borrowed facts (network, our work, our length), so it needs no chain to
  mutate and can run with the caller's lock released.
- **`Chain::adopt_reorg`** — cheap, safe under the lock, and it *repeats* the
  work comparison rather than trusting the earlier verdict. That re-check is
  what makes releasing the lock sound: our chain may have moved past the
  candidate while it was being verified, and adopting blindly would roll the
  node backwards onto a chain the network had already left.

Both node call sites now copy three numbers under the lock, release it, verify,
and come back only to swap. A `ReorgFlight` guard admits one verification at a
time, so peers that noticed the same divergence no longer each pull a whole
chain over the wire to prove the same point. It clears on drop, so a panic or
an early return cannot leave reorgs disabled for the life of the process.

## Tests

141 passing, up from 138. The three new ones cover the risk the split
introduces rather than the happy path:

- `evaluating_a_reorg_needs_no_chain_to_mutate` — keeps the expensive half
  free of `&mut self`, which is the whole reason it can leave the lock.
- `a_candidate_that_lost_the_race_is_not_adopted` — a chain that overtook the
  candidate mid-verification must not be rolled backwards.
- `splitting_the_reorg_did_not_change_the_verdict` — both routes reach the
  same decision, so the refactor moved a lock and not a consensus rule.

`cargo fmt --check` and `clippy --workspace --all-targets -D warnings` clean on
the toolchain CI uses.

## Measuring it yourself

```bash
cargo run --release -p nightfall-node --example reorgcost -- <blocks.jsonl>
```

Prints what a full-chain reorg costs on your own hardware and chain.

## Still true

None of the tests start two nodes. This bug was found by instrumenting a live
network, not by the suite, and the same is true of the four fixed in v0.5.3. A
multi-node integration harness remains the next piece of work.

## Downloads

`NIGHTFALLCOIN-Core-0.5.4-macOS-arm64.dmg` · Apple Silicon, macOS 11+
`NIGHTFALLCOIN-Core-0.5.4-macOS-intel.dmg` · Intel, macOS 10.15 Catalina+
`nightfall-core-0.5.4-windows-x64.exe` · Windows 10+ — the wallet
`nightfalld-0.5.4-windows-x64.exe` · Windows — headless node
`nightfall-wallet-0.5.4-windows-x64.exe` · Windows — command-line wallet

Verify against `SHA256SUMS-0.5.4.txt` and `SHA256SUMS-0.5.4-windows.txt`.
