# Release guidance consistency

## Purpose

Prevent stale installation instructions from silently
surviving a new Core release.

This is a distribution-assurance utility aligned with
the roadmap's reliability and independent-operation goals
and the whitepaper's authenticated-distribution work.

## Offline tests

    python3 -m unittest discover -s scripts \
        -p test_release_guidance.py -v

## Public observation

    python3 scripts/check_release_guidance.py \
        --report evidence.json

## Exit status

- 0: two consistent observations without detected drift.
- 1: two matching observations contain detected drift.
- 2: inconclusive, unavailable or ambiguous evidence.

## Scope

The checker compares:

- Latest stable GitHub Core release.
- Homepage Core version.
- Build guide checkout version.
- Published checksum manifest names.
- Documented minimum Core version 1.1.1 required
  for the activation at block 500,000.

The compatibility floor is a documented historical
requirement, not an automatically discovered future
consensus requirement.

Historical release documents and whitepapers are not
required to match the newest release.

## Limitations

This tool checks public release guidance and manifest
references, not artifact bytes, signatures, link
reachability or cryptographic supply proofs.

A publication delay or intermediary cache may produce
temporary drift. Two observations do not eliminate every
possible cache inconsistency.

Unexpected HTML layouts are classified as inconclusive.

Do not interpret a drift report as a verified consensus
vulnerability.

Use the existing release provenance checker to verify
downloaded artifacts independently.

## Privacy

No wallet access, private keys, recovery words, mining,
chain mutation, credentials or transaction submission.

Only public HTTPS resources are queried during an explicit
live observation. Offline unit tests require no network.

No telemetry is added to any wallet or node.

## Release checklist

1. Publish the intended stable Core release.
2. Update active installation guidance and manifest links.
3. Run the public observation.
4. Investigate drift before announcing the rollout.
5. Preserve historical release and whitepaper versions.

This utility does not automatically modify the website.

## Review scope

No Rust source, consensus rules, monetary parameters,
wallet formats or existing CI jobs are modified.

## Automated monitoring

The workflow runs daily at 04:17 UTC after merging into the default branch.

Manual execution remains available.

Scheduled runs require public HTTPS access. Drift and inconclusive observations produce non-successful workflow runs for maintainer investigation.

No automatic website or release changes are performed.
