# Nightfall Reliability Lab

Status: local, 30 September 2026 (Roadmap 2027, milestone 5).

One command starts real `nightfalld` processes on devnet, on loopback, in
private temporary directories, and puts them through what unit tests cannot:

```
cargo +1.98.0 build --offline --locked --bins -p nightfall-node -p nightfall-nodekit
node scripts/lab/run-lab.mjs            # report in output/lab/<timestamp>/
```

| Scenario | What must hold |
|---|---|
| Fresh node syncs from a peer | Empty node connected to a node with 20 mined blocks reaches the same tip hash |
| Clean restart | Tip identical after SIGTERM and restart; `nightfall-nodekit doctor` reports no failure |
| Hard kill | After SIGKILL, doctor recognises the lock record as a dead pid (not "held"), node restarts, no blocks lost |
| Partition and heal | Two nodes with common history each mine alone (2 vs 5 blocks); once connected, the lighter side reorganises onto the heavier branch's exact tip |
| Late joiner | A node still holding the pre-partition chain follows the winning branch, not its own memory |

Every verdict comes from the nodes' own `/status` answers and the Node Kit's
own diagnosis. The report (`report.json`, `report.md`) records commit,
binary version, platform, heights, tip hashes and timings — no keys, seeds or
addresses.

## Guard rails

Devnet only (it has no seed list), SOCKS proxy off, explicit `--datadir` in a
fresh temp directory, every listener on 127.0.0.1, random port block
28000–32990. Temp directories are deleted after a clean run and kept, with
logs, after a failure.

## Results on 30 September 2026

Four consecutive runs on darwin/arm64, commit 712e138 plus the lab script:
**5/5 scenarios passed each time** (≈12–17 s per run). Converged tips and
heights are in the reports under `output/lab/` (local, not committed).

## Not covered — say so, don't imply it

- different machines, operating systems, real latency or packet loss;
- mainnet-sized chains and long-running soak;
- adversarial peers (malformed messages, eclipse attempts);
- faults beyond a process kill — disk full, power loss mid-write; the kill
  here lands on an idle node, not mid-write;
- a live link being cut: the partition is made by keeping nodes apart while
  they mine, then connecting them.

The roadmap's broader reliability goal (field evidence from operators on
real hardware) is external and not evidenced by this lab.

## 11 October extension

Added a bounded five-cycle SIGTERM/SIGKILL restart regression with exact-tip and doctor checks, plus a throttled loopback peer link so SIGKILL interrupts incomplete sync while response bytes are arriving. Recovery must reach the original exact tip and pass doctor. This is active-sync interruption coverage; the exact filesystem-write instruction is not instrumented and multi-day soak remains a practical test scope, not a claim. `NIGHTFALL_LAB_BIN_DIR` selects the release binaries to test.
