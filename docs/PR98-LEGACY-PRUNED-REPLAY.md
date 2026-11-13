# PR98: Versioned legacy-pruned migration regression

## Pinned releases

- NightfallCoin v1.1.2: 7d336540916b96ec7a6679d868106e628c0bee93
- NightfallCoin v1.1.3: b0c83258ac83972b40a5b5dbba1c987ae027db98

The test uses the original storage and consensus implementations
from the pinned official release commits.

Each release generates a synthetic 14-block Devnet pruned
data directory. It retains four blocks and prunes ten.

This intentionally small fixture checks compatibility and safety.
It is not representative of a production 500-block prune window
or the size of a real Mainnet installation.

## Verified behaviors

1. Refusal without --force must preserve chain and identity bytes.
2. The updated node must refuse old unsealed pruned storage.
3. A wrong-network request must refuse without chain mutation.
4. An interrupted recovery journal must prevent startup.
5. Explicit recovery operates only on a disposable fixture copy.
6. The old chain backup must be byte-identical.
7. Synthetic wallet and miner identity files must survive.
8. The reset fixture must reload successfully at Devnet genesis.
9. Existing Node Kit refusal and Rust recovery tests must pass.

## Reproduction

Run inside the PR98 working copy:

    python3 scripts/pr98_legacy_pruned_release_replay.py --workspace .

This script automatically clones the pinned releases into
temporary directories and generates synthetic fixtures using
their actual original Rust libraries.

## Remaining maintainer acceptance gates

The following are NOT proven by this regression:

- Untouched real-world 1.1.2 operator pruned data directory
- Mainnet-sized startup performance
- Actual Mainnet checkpoint export/import/catch-up
- Independently reproduced kernel, UTXO, supply and work equality
- Full production interrupted-import behavior
- Reviewed compiled checkpoint and maintainer approval

The tested forced recovery intentionally preserves a backup and
resets the synthetic chain to genesis. It is not an in-place
trust-preserving migration.

No live miner, wallet or blockchain is modified.
