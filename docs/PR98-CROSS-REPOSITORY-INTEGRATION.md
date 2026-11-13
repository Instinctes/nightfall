# PR98 selective cross-repository consolidation

Main PR: https://github.com/Instinctes/nightfall/pull/98

## Non-consensus Merkle-sum research

Source: https://github.com/nightfall-wizard/nightfall/pull/3
Pinned SHA: `7f92c6e92c43133a31ba3a46d3657d8078ffafec`

Only the research module and its documentation were transplanted.

The root is not an authenticated protocol-v8 mainnet UTXO root.

No global-supply certificate is claimed.

## Owner-gated agent intentions

Source: https://github.com/nightfall-wizard/nightfall-wizard-fork/pull/4
Pinned SHA: `881e45c08ade631df851199f87496bc5211c3ef1`

Only the agent policy, manual-approval CLI handler, and its
security-boundary documentation were transplanted.

No unattended signing, automatic payment, or consensus change.

## Integration and acceptance boundaries

Old fork histories were intentionally not wholesale-merged.

Existing #17, #27, #94 and #95 are already consolidated in #98.

Outstanding acceptance:

- untouched 1.1.2 pruned fixture
- upgrade refusal/migration and interrupted restart
- mainnet-sized benchmarks
- real checkpoint export/import/catch-up
- supply/root/work equivalence
- interrupted multi-file import
- reviewed checkpoint selection
- post-height-500000 maintainer approval

Local unit tests and green CI cannot replace these production gates.

No running node, local blockchain, wallet or miner is changed.
