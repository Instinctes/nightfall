# Nightfall Network Readiness Evidence

## Purpose

Nightfall 1.1.1 activates a consensus upgrade at block 500000.

Older nodes and miners must upgrade.

This tool provides public-only network readiness evidence.

It uses Python's standard library.

It does not install a wallet or miner.

It does not modify consensus.

## Commands

Run offline tests:

    python3 -m unittest discover -s scripts -p test_network_readiness.py -v

Run public endpoint verification:

    python3 scripts/network_readiness.py --report readiness.json

Compare public snapshots:

    python3 scripts/network_readiness.py --state readiness-state.json

Evaluate a saved JSON response:

    python3 scripts/network_readiness.py --input saved-network.json

## Exit codes

0: Valid report or advisory warning.

2: Public accounting or chain identity contradiction.

3: Network, input or parsing failure.

## Verification

The checker independently calculates scheduled emission.

It checks:

- Mainnet genesis identity
- Protocol and wire version
- Block count and height agreement
- Monetary emission schedule
- Integer halving boundaries
- Maximum supply
- Minted versus burned amounts
- Circulating supply accounting
- Reported supply invariant status
- Difficulty floor
- Block hash encoding
- Cumulative work encoding
- Implausible timestamps
- Legacy peer version labels
- Consensus activation proximity
- Public chain regression observations

## Trust limitations

This is not independent cryptographic verification.

A dishonest node can fabricate internally consistent JSON.

The checker does not validate historical blocks.

It does not verify Bulletproofs or Schnorr signatures.

It does not establish the true network-wide node count.

Version observations are not independent node identities.

The actual supply proof requires a validating node.

## Privacy

No wallet data is collected.

No seed phrases are collected.

No IP address tracking is performed.

No user analytics are implemented.

No RPC credentials are required.

The tool accesses only a fixed public HTTPS endpoint.

Redirects are refused.

Responses are size-limited.

Local comparison state contains only public chain information.

## CI

The supplied GitHub Actions workflow runs offline.

There are no external service calls from the test suite.

## Maintainer acceptance

The contribution is complete for review when:

1. All adversarial tests pass.
2. The CI workflow passes.
3. A current public endpoint sample is documented.
4. No private information is collected.
5. Trust limitations remain explicit.
6. No consensus or wallet code is changed.

## Next development

Implement a separate Devnet multi-node failure-testing harness.

Scenarios should include:

- Network partition
- Reconnection
- Competing chain selection
- Interrupted synchronization
- Restart during archive persistence
- Fork recovery
- Supply consistency after recovery

All tests must use disposable Devnet data.

No production mainnet modifications.
