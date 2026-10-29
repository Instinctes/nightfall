# Release activation source-identity guard

This read-only check pins protocol v8, wire v6,
the existing genesis, mainnet binding activation
at height 500,000, legacy block-record paths,
and names of upstream regression tests.

The checker includes seven mutation checks.

Future intentional protocol upgrades must
update this contract.

Run:

    python3 scripts/check_activation_contract.py --selftest

This is not a consensus test, an audit,
or proof of actual behavior.

Keep running the Rust workspace, consensus
and storage suites separately.

No live mainnet datadir, private key,
seed, wallet or miner is accessed.
