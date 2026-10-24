# CLI backup recovery rehearsal (disposable Devnet only)

The roadmap asks for repeatable backup recovery exercises. This narrowly scoped
regression checks the **CLI seed-file boundary**, not the Core encrypted vault,
not BIP-39 word recovery, and not a real funds-recovery guarantee.

Run using a locally built wallet:

```sh
cargo build --locked -p nightfall-wallet
python3 scripts/test_cli_backup_rehearsal.py --wallet target/debug/nightfall-wallet
```

The runner accepts **only** the path to an executable. It creates a new private
temporary directory, generates a disposable **Devnet** wallet, and then checks:

1. A copied seed plus local state reopens to exactly the same address.
2. Missing seed with existing wallet database is rejected without generating a new seed.
3. A truncated seed is rejected without altering the damaged file.
4. A different, syntactically valid seed produces a different address and is detected by comparison.
5. Interrupted database writes are not silently cleared.
6. A seed-only backup recovers key identity but not automatically all historical payments.

The only wallet commands used are `init`, `address`, and `balance`, with RPC
set to the unusable local endpoint `127.0.0.1:0`. The test never connects to
Mainnet, performs a payment or reads any existing wallet directory. Temporary
seeds are never printed, committed or uploaded; disposable files are removed
at the end of each test. The normal project CI is separate; an opt-in workflow
also builds the CLI and runs this exercise on relevant pull requests.

**Do not supply a real seed or datadir to the test.** Never use its generated
addresses to receive funds. A complete user-facing Backup Check must separately
cover encrypted Core/browser vaults, mnemonic validation and a verified chain
rescan; those are explicitly outside this PR.
