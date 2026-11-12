# NIGHTWATCH: public evidence without tracking users

Status: experimental read-only tool.

Source:
https://nightfallcoin.org/network.json

## Roadmap relevance

This contribution addresses measurable network
reliability and post-activation operational evidence.

It:

- Records bounded public network observations.
- Recalculates reported issuance using integer darks.
- Checks the fixed mainnet identity.
- Detects contradictory supply accounting.
- Detects height and cumulative-work regressions.
- Flags obsolete reported software versions.
- Builds a local observation history.
- Produces JSON, Markdown and HTML evidence.
- Includes adversarial offline tests.
- Does not access private wallet information.

## Commands

Single observation:

python3 scripts/nightwatch.py

Continuous observations:

python3 scripts/nightwatch.py --watch --interval 300

Offline evidence integrity check:

python3 scripts/nightwatch.py --verify

Regression tests:

python3 -m unittest discover -s scripts -p test_nightwatch.py -v

Outputs:

scripts/output/

These generated observations should not be committed.

## Security boundaries

No RPC administration.

No mining.

No transaction signing.

No wallet secret access.

No wallet addresses.

No user activity tracking.

No IP-level tracking.

No consensus rule changes.

## Explicit evidence limitations

The public endpoint is provided by a node.

This tool does not independently validate
the chain, its cryptography or proof of work.

The node can report false information.

SHA-256 journal linking is not an
authenticated digital signature.

Peer version labels do not identify
unique independent operators.

The public endpoint does not provide
reliable mining-pool attribution.

Consequently, mining concentration
must remain UNKNOWN.

## Nightpool feasibility

Nightpool is an important long-term
decentralization objective.

The current ledger enforces exactly
one coinbase output per block.

Reference:

crates/nightfall-ledger/src/lib.rs

A noncustodial multi-recipient payout design
cannot simply be installed as a script.

It requires a reviewed architecture,
explicit consensus-compatibility analysis,
Devnet evidence and adversarial tests.

No mainnet payout change is proposed here.

## Next acceptance milestone

Run this observation tool against
independently administered sources,
with explicit source trust boundaries.

Demonstrate post-activation reliability
and report discrepancies openly.

Obtain privacy-safe public pool attribution
before claiming measured concentration.

Independent cryptographic assurance
remains a separate requirement.

## Post-activation upgrade evidence

The activation at height 500000 requires
consensus-compatible node software.

Release 1.1.1 introduced the activation.
Release 1.1.2 maintains those rules.

The Upgrade Gate classifies reported
versions without identifying node operators.

The report distinguishes:

- Known incompatible legacy labels.
- Confirmed compatible 1.1.1/1.1.2 labels.
- Unknown newer software labels.
- Legacy observations across time.
- Possible discrepancies in version totals.
- Possible source-tip stagnation.

No version label proves that its node
validates the chain correctly.

No peer is contacted individually.

No automatic upgrades take place.

Run:

python3 scripts/nightwatch_upgrade_gate.py

Test:

python3 -m unittest discover -s scripts -p 'test_nightwatch*.py' -v

Evidence:

scripts/output/upgrade-gate.json
scripts/output/upgrade-gate.md

The outputs are local operational data,
not files intended for source control.

Next requirement:

Independent full nodes must validate
the post-activation chain.

Only independent node evidence can
substantiate claims about decentralization.

## Automatic upgrade integration

The Upgrade Gate now executes after each
successful public-network observation.

It uses the same validated observation
journal as the primary Nightwatch report.

No additional network request is required.

A separate integration regression test
confirms the generated upgrade report.

The monitor remains read-only.

Unexpected upgrade evaluation failures
are logged and do not erase the journal.

No automatic remote node modifications,
wallet operations or consensus changes.
