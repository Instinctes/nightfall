#!/bin/sh
# H7 — a repeatable happy-path harness. Does not touch mainnet.
# Needs bitcoind + bitcoin-cli on PATH. Nightfall testnet is not started here;
# this script only documents the Bitcoin half against the existing
# `cargo test -p nightfall-swap --test regtest -- --ignored`.
set -e
echo "Use: cargo test -p nightfall-swap --test regtest -- --ignored --nocapture"
echo "That test starts bitcoind in /tmp/nfregtest on port 18999."
echo "A full two-process NIGHT+BTC swap is not wired: no transport server"
echo "by design. Packets are copy-paste (nightfall_swap::packet)."
exit 0
