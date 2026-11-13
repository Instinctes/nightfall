# Nightfall unified integration — review handoff

UTC: 2026-11-13T09:08:37+00:00
Upstream base: `b0c83258ac83972b40a5b5dbba1c987ae027db98`
Contributors: `nightfall-wizard`

## Inputs
- #17: storage: authenticate pruned state with semantic seals — `36fe029324ef3da06083cb775c0d41eba4bb6190`
- #27: storage: add checkpoint-state bootstrap — `333cbe352ce7404b6d24ae5712929898227c9bc5`
- #94: test: public network activation and supply readiness evidence — `856a415e786b888070a353b87a96dad6fccd667e`
- #95: test: longitudinal Nightfall network and upgrade evidence — `59f542d4be43891b56f9b3899ce9d49ec50b18f2`

## Strategy

Merge PR #17 using `-X theirs`, then PR #27
using `-X ours`, then #94 and #95 using
`-X ours`.

These options select conflict hunks only;
they are not proofs of semantic compatibility.

Preserve upstream `is_link_like` in `vault_fs.rs`:
main already checks Windows reparse points and
symlinks and on Unix symlinks.

The overlapping PR #95 refactor does not
add another protection.

The integration gate checks the full bytes of
newly added files, important added source lines,
and all uniquely named storage unit tests from
PR #17 and PR #27.

Passing these checks or CI is not a proof of
full behavioral equivalence.

## Maintainer acceptance STILL BLOCKING

- Untouched 1.1.2 pruned datadir fixture
  and explicit upgrade UX
- Crash/restart and migration refusal safety
- No trust of legacy pruned state based only
  on locally calculated hashes
- Pruned mainnet-sized startup benchmark
- Real checkpoint export -> import -> catch-up
- UTXO root, kernel sum, supply and work equality
- Multi-file import interruption testing
- New compiled checkpoint and documentation
- Post-block-500000 maintainer review

**Ready for code review, not asserted production safe.**

No consensus, genesis, supply, chain, datadir,
mining or wallet activation was performed.

Public-endpoint evidence is not independent
consensus validation.
