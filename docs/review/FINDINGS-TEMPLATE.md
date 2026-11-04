# Finding NF-YYYY-NNN — <one-line title>

| Field | Value |
|---|---|
| Reporter | name / handle (or "anonymous") |
| Date reported | YYYY-MM-DD |
| Source revision | commit hash the finding was made against |
| Component | crate / file / function (e.g. `nightfall-ledger/src/utxo.rs::apply`) |
| Severity (reporter) | Critical / High / Medium / Low — see EXTERNAL-REVIEW-PACKAGE.md §5 |
| Severity (maintainer) | filled in on triage |
| Status | reported → confirmed / not reproducible / by design → fixed in <commit> → verified by reporter |

## Summary

Two or three sentences: what is wrong and what an attacker gains.

## Affected assets

Which of: supply integrity · spend authority · privacy · chain agreement ·
availability. And which users: all nodes / light wallets / browser wallet /
merchants / operators.

## Preconditions

What the attacker needs (hashrate share, a peer connection, a local account,
a user click, a malicious light-API node …).

## Reproduction

Exact steps on **devnet** (never mainnet), with commands and expected vs.
observed output. A failing test is the best reproduction:

```
cargo +1.98.0 test -p <crate> <test_name>
```

## Impact

What happens if exploited, and how it would be noticed (or not).

## Suggested fix

Optional.

## Maintainer response

- Triage date and assessment
- Fix commit(s) and the regression test that locks it (tests that invert a
  working exploit go next to `crates/nightfall-ledger/tests/exploit_regression.rs`)
- Disclosure: date and wording agreed with the reporter
