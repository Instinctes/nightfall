# Nightfall Upgrade Assurance Gate

## Purpose

Read-only pre-activation checks against the public
Nightfall network status document.

This tool does not run a node, hold wallet keys,
send payments, mine, or modify chain state.

It does not independently verify consensus or PoW.

## Run

    python scripts/nightfall_upgrade_gate.py --json

    python scripts/nightfall_upgrade_gate.py

    python -m unittest discover -s scripts -p 'test_nightfall_upgrade_gate.py' -v

## Exit status

0 = PASS

1 = WARN

2 = FAIL

A stale, invalid, or unavailable status source
fails closed.

Legacy reported node versions produce warnings.

## Acceptance criteria

1. Mainnet genesis must match the v8 genesis.

2. Network, protocol and wire versions must match.

3. The source must report a valid supply invariant.

4. Minted supply minus burned fees must equal
   reported circulating supply.

5. Minted supply must not exceed the hard cap.

6. Within the first subsidy era, reported issuance
   must agree with the six-NIGHT block subsidy.

7. The reported tip must be recent.

8. Node versions below 1.1.1 must be flagged
   before the activation at block 500000.

9. No wallet addresses, peer IPs, recovery words
   or private keys are processed.

## Security limits

The checks establish only internal consistency
of data reported by the website's node.

The tool does not establish that the source node
is honest or that its chain is valid.

Peer-version observations do not establish
network-wide upgrade adoption.

The tool does not replace independent security
review, cryptographic validation, or real
multi-node integration tests.

## Compatibility

No consensus changes.

No wallet format changes.

No network protocol changes.

No release binary modifications.

No mainnet state changes.

Only new Python source, regression tests and
documentation are contributed.
