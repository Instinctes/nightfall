# Independent Node Evidence — redacted operator evidence (experimental)

This tool supports the [September 2027 roadmap](https://nightfallcoin.org/roadmap/)
independence checkpoint. This evidence probe is separate from the
`nightfall-nodekit` operator crate and `docs/NODE-KIT.md`. It collects **local, read-only RPC claims**
into a tightly allowlisted, hash-sealed JSON file. It neither reads wallet
files nor modifies nodes, peers, mining, chain state or keys.

## Operator use

Run on the machine operating `nightfalld` (mainnet, protocol 8, wire 6):

```sh
python3 scripts/test-node-evidence.py
python3 scripts/node-evidence.py capture --output local-node.json
```

**Version compatibility:** This probe accepts protocol 8 / wire 6 exactly.
When either protocol version changes, update the version checks, test
fixtures and this documentation before using the probe again.

If the local daemon is not running, the probe fails clearly. Exit status is
0 for ordinary observations, 1 for detected cautions and 2 for failure.
Repeat on another **independently managed machine**; exchange only the
redacted receipts, then compare offline:

```sh
python3 scripts/node-evidence.py compare local-node.json other-node.json
```

Same height with different reported tip hashes is a meaningful discrepancy;
different heights are **inconclusive**. A matching tip is not proof of independent
infrastructure, node honesty, consensus validation or coin supply.

## Trust and privacy boundaries

- The tool talks only to `127.0.0.1` at a configurable port. Nightfall's local
  unauthenticated RPC must not be forwarded or exposed to the Internet.
- It calls only `status` and `verify_supply`. The latter invokes the **node's**
  supply check, not a second mathematical implementation. A malicious local node
  could fabricate both replies; the evidence must **never** be called a ZK proof,
  audit, certification, remote attestation or security guarantee.
- A SHA-256 seal detects accidental edits (and edits without recomputing the
  hash), but it is not a digital signature and offers no author authentication.
- Peers' IP addresses, wallet balance, scan history, seed phrases, RPC credentials,
  miner addresses and transaction identifiers are deliberately absent.
- Running twice can reveal ordinary reorgs. Do not mine if peers are absent or
  your chain is behind. This program takes no automatic recovery action.
- For reproducible comparison, only schema-v1 allowlisted fields are accepted.
  Do not commit live receipts to a public repository without reviewing them.

## Scope and next independent acceptance test

The maintainer can review this separately from pending consensus, P2P and
crypto PRs. Completion of the roadmap's independence criterion still requires
unrelated operators to independently install/synchronize nodes and exercise
devnet payments/recovery. This code proves none of those activities occurred.
