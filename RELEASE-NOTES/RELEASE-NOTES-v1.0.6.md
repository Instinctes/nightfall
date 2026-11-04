# NIGHTFALLCOIN 1.0.6 — nodes that rejoin, payments that resolve

1.0.6 fixes the failures behind issue #40 (a node stuck on a fork for 19 hours),
issue #24 (a 5,000 NIGHT payment that never confirmed) and a community report
of Core sitting on "Checking the current chain" with a payment pending for
days. Same chain, genesis, protocol v8 and wire v6; no consensus rule changed.

## Nodes no longer fork themselves, and rejoin when they do

- **No mining while catching up** (#40, PR #43 by @parthod0x). The hold-off that
  should stop a node from mining while a peer is known to be ahead expired after
  ten minutes even during a normal sync. A node then mined at the low historical
  difficulty of its own stale tip — 1,491 blocks during one resync on mainnet —
  and forked itself onto a private chain. The hold now lasts as long as a fresh
  peer is ahead; an isolated node still starts mining once its peers go quiet.
- **Fetching the peer's chain at all** (#40). To leave a fork a node opens a
  fresh connection and asks the peer for blocks. The peer treats that
  connection like any session and first sends its address book (twice) and
  status probes; 1.0.5 read the first of those as the reply and gave up with
  "expected blocks". The request now waits past unrelated messages.
- **Finding where two chains split** (#40). A node far behind its peer searched
  for the common block starting at the *peer's* tip. Once the peer was more
  than about 500 blocks ahead, that search never reached a height the node had,
  and every chain was rejected as "unexpected height" — from every peer, for
  ever, even for a three-block fork. The search now starts at the lower of the
  two tips and narrows down to the exact block.
- A new test runs two real nodes, 580 blocks apart, the lagging one on a
  three-block fork and not reachable from outside (as the #40 node, on WSL2,
  was not). With 1.0.5 code it stays stuck — first on "expected blocks", and with
  only that fixed on "unexpected height", exactly as in the report. With 1.0.6
  it rejoins. (It mines 620 real blocks, so it is run by name rather than in
  CI: `cargo test --release -p nightfall-node --test far_behind_fork -- --ignored`.)
- **"Dead branch" means a dead branch.** 1.0.5 called a node on a dead branch
  whenever it was stuck *and* far behind, and reported the lag as the depth of
  the fork. Only a real rewind past the 500-block limit is called that now.

## Core: a way out, and payments that resolve

- **The reason is shown again.** A node on a real dead branch is explained on
  the startup screen and the dashboard, with **Resync chain, keep wallet** and a
  confirmation. A node that keeps refusing the network's chain for three minutes
  for any other reason gets the same explanation instead of an endless spinner.
- **Resync no longer needs a rescan.** The chain file is moved into a backup
  folder and downloaded again; the wallet is not reset and pending payments no
  longer block it. Once the node has caught up, the background scan reconciles
  the wallet automatically — also when the abandoned branch was taller than the
  network's chain.
- **Payments that can never confirm release their coins.** When the wallet is
  fully scanned to a node that has caught up with its peers, a pending payment is
  removed — and its remaining coins become spendable again — if one of its inputs
  is not in the chain's unspent set (spent elsewhere, or mined on an abandoned
  branch), or if the chain refuses the transaction itself. Payments younger than
  ten minutes, payments waiting for coinbase maturity and valid payments that are
  simply not mined yet are never touched. Core says so once.
- **Large payments from many small coins** (#24, PR #25 by @nightfall-wizard). A
  transaction may spend at most 512 inputs. A wallet holding mostly 6 NIGHT mining
  rewards built a 5,000 NIGHT payment from ~830 of them, which no block may
  contain. The wallet now refuses that up front and says to send in parts;
  payments already stuck this way are released as described above.

## Hardening from community pull requests

Integrated after review: untrusted peer heights at `u64::MAX` can no longer
overflow sync, reorg-fetch and pagination arithmetic (PRs #11, #13, #16), the
prune boundary no longer overflows (#14), a chain file ending in a truncated
record-length prefix is rejected instead of accepted (#26), and new tests cover
the reorg-depth boundary and reorg reload state (#5, #6). Thanks to
@nightfall-wizard and @parthod0x.

## Upgrade

No chain reset is required and the wallet file format is unchanged. Quit Core
fully before replacing the application, and keep an encrypted backup. Node
operators: replace `nightfalld` and restart; a node stuck on a fork usually
rejoins by itself after the upgrade.

**If 1.0.5 is stuck right now:** install 1.0.6 first. If it does not recover on
its own, use the resync offered on the startup screen. By hand: quit Core, open
the data folder (`~/Library/Application Support/nightfall/mainnet/n8` on macOS,
`%APPDATA%\nightfall\mainnet\n8` on Windows, `~/.local/share/nightfall/mainnet/n8`
on Linux), move `blocks.bin` and `chain-meta.json` into a new backup folder,
create an empty `blocks.bin`, and start again. Seed and wallet stay; coins mined
on an abandoned branch are gone.

Downloads include macOS arm64/Intel DMGs, Windows/Linux Core, node and CLI wallet
binaries, plus platform SHA256 lists. macOS bundles are ad-hoc signed; they are
not Developer-ID signed or notarized. Windows binaries have no publisher
certificate.

Validation: workspace tests and Clippy on Rust 1.98.0; two-node far-behind-fork
test (run against 1.0.5 code as a control: it fails there); isolated Devnet test that mines on a branch, spends
from it, resyncs onto a different, shorter chain and checks that the wallet
rebases and the impossible payment is released; layout tests for the new notice
at 620/884/1180 px. This is not a claim that every possible defect has been
ruled out.
