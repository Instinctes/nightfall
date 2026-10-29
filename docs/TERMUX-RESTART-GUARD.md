# Termux Mainnet v8 Restart Guard

## Purpose

Read-only safety preflight for existing
NightfallCoin mainnet v8 installations.

Detects:

- Missing blockchain files.
- Wrong genesis.
- Wrong network.
- Wrong protocol.
- Genesis-only state.
- Interrupted recovery.
- Conflicting metadata.
- Missing miner seed.
- Unsafe seed permissions.

## Safety

The guard never:

- Deletes blockchain data.
- Resets a node.
- Executes fork recovery.
- Creates replacement wallet seeds.
- Stops mining.
- Broadcasts transactions.
- Changes consensus.
- Sends telemetry.

## Limitations

This is local metadata inspection.

READY does not prove valid proof of work,
correct supply, synchronization or balances.

An active daemon may change the files
during inspection.

A REVIEW result is not proof of corruption.

Never reset blockchain data based solely
on this report.

## Testing

Run the synthetic Python regression tests
using unittest discovery.

All test data is disposable.
