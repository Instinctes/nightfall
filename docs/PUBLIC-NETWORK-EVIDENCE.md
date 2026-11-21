# Public network evidence gate (experimental)

This is a bounded, read-only check of what **nightfallcoin.org reports**. It
is not a full node, a cryptographic verifier, a security audit, a wallet
health check, or evidence that a public service is independently operated.

## Why this is separate

- `scripts/node-evidence.py` checks **local** node RPC claims, with a distinct
  trust boundary and optional comparison of separately collected receipts.
- `scripts/check-emission.mjs` checks the website's **static UI** and its
  emission calculation; website code is deployed separately.
- This script checks the **live public website contract** against exact integer
  protocol emission and against the site's three published supply routes.
  It does not duplicate a mining, wallet, or consensus implementation.

## Commands

From the repository root, on Termux or any host with Python 3:

```sh
python3 scripts/check_public_network_status.py --self-test
python3 -m unittest discover -s scripts -p 'test_public_network_status.py' -v
python3 scripts/check_public_network_status.py --report network-evidence.json
```

No packages beyond the Python standard library are required. The first two
commands make **no network requests**. The third uses read-only HTTPS GET
requests to `https://nightfallcoin.org/network.json` and
`/api/supply/{max,total,circulating}`. No RPC credentials, wallet keys,
telemetry identifiers, transaction broadcasting, or node access are involved.

The script checks reported mainnet identity, protocol and wire versions,
block count versus tip height, supply-ready flag, integer subsidy arithmetic
(6 NIGHT initially, 7,500,000-block halving periods, 90,000,000 NIGHT cap),
reported burns and circulating units, decimal supply API values, tip stability
and timestamp freshness. It uses **integer atomic units** throughout.

## Three outcomes

| Exit | Result | Interpretation |
| --- | --- | --- |
| 0 | `CONSISTENT` | Public data matched in one bounded observation window. |
| 1 | `MISMATCH` | The reported data violated a documented consistency rule. Investigate; this does not prove a consensus failure. |
| 2 | `INCONCLUSIVE` | Node loading, changed tip, network error, or stale timestamp prevented a reliable comparison. Do not infer a monetary mismatch. |

Three short attempts handle ordinary tip movement or transient availability.
An unexpected mainnet protocol or wire upgrade deliberately fails closed:
review the checker's assumptions alongside new protocol documentation.

## GitHub Actions

`.github/workflows/public-network-evidence.yml` runs offline tests on matching
pull requests and commits. A daily, *separate*, scheduled job (and optional
manual dispatch) records the live observation in a short-retention artifact:

- `MISMATCH`: red job, requiring human investigation.
- `INCONCLUSIVE`: neutral completed job with explicit warning in its log.
- `CONSISTENT`: green job with data and timestamps in the artifact.

The live observation is **not** a merge gate; outages cannot break offline PR
validation. This workflow never pushes code, fixes chains, submits transactions
or changes the website. The scheduled job exists only after the maintainer
merges the workflow into the default branch and may be disabled independently.

## Limitations and safety

Public endpoints can report mutually consistent **false claims**. A hash in a
JSON response is not a verified block header. The site's `supply_invariant_ok`
field is a claim from the service, not an independent evaluation of
`Σ UTXO − Σ kernel_excess = (minted − burned)·G`. Network partitions, CDN
caching, upgrades and reorgs can change observations. A flagged discrepancy
should first be reproduced with separate full nodes before treating it as a
chain vulnerability. Never share private wallet or transaction information
in an issue or report. Coordinate sensitive consensus/supply findings through
the repository's private security contact, not a public issue.
