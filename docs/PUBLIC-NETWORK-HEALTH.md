# Public Network Health

## Status

Roadmap contribution: privacy-preserving public network health.

This component observes published public node data.

It is not an independent blockchain audit.

## Execution

    python3 scripts/test_public_network_health.py

    python3 scripts/public_network_health.py --samples 2 --interval 3 --output health.json

## Security boundaries

The collector accesses only:

https://nightfallcoin.org/network.json

It never accesses wallet storage, recovery words,
private keys, local RPC or mainnet peer connections.

The response size is limited to 64 KiB.

Duplicate JSON keys and nonfinite values are rejected.

Network identity, genesis, protocol and wire versions
must match the documented mainnet.

Reported monetary accounting is checked for
internal consistency.

The public node's supply-invariant claim is reported,
not independently cryptographically verified.

Unrecognized fields are removed from output.

Peer version labels are strictly validated.

No individual peer IP addresses are collected.

The public HTTPS server still sees the requesting
client's connection IP.

## Evidence

Reports contain only selected public fields.

Reports contain a SHA-256 checksum.

The checksum protects against accidental modification.
It is not a digital signature or proof of consensus.

A dishonest endpoint can publish false information.

A CDN may return cached data.

Version aggregates do not establish unique node counts.

A shorter chain can legitimately have more total work.

A stale reported tip does not alone establish
a consensus failure.

## Exit behavior

Exit 0: valid report generated.

A CAUTION assessment can still return exit 0.

Exit 2: public source unavailable or invalid.

Offline CI tests never access the live network.

## Compatibility

No changes to:

- Consensus
- Proof of work
- Supply rules
- Emission
- Wallet files
- Recovery words
- Miner state
- Mainnet blockchain
- Protocol version
- Wire version

## Roadmap relevance

Provides a small, testable foundation for
the proposed Network Health Live feature.

It does not replace external security auditing.
