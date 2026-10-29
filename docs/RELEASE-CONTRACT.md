# Nightfall v8 release contract

This read-only source/documentation policy check pins
protocol 8, wire 6, mainnet genesis, height-500000
activation, and supply and halving parameters.

Run: python3 scripts/release_contract_guard.py --test

Five negative regression tests deliberately corrupt
copies in a temporary directory.

They never change the checked-out sources or running
node data.

This is a policy check, not cryptographic proof,
a Rust consensus test, network replay, or an
independent security audit.

A deliberate consensus rule change requires
a separately reviewed contract update.
