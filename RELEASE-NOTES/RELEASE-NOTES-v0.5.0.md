**The chain has been reset.** Two networking faults, both introduced earlier the
same day, between them stopped the network and made a confirmed payment
disappear. Neither made the chain invalid — they made it untrustworthy, which
is worse, because nothing reported anything wrong.

```
protocol   v7 · wire v4
genesis    c8614333c0f86a4824df212474632f4b9feecf9bf0593841199d894127f2f9a6
```

Delete any earlier data directory. Coins mined before this do not carry over;
there is no migration and there was never a claim there would be.

---

## What went wrong

**The mining hold-off deadlocked the network.**

Mining paused while a peer reported a greater height, so that nobody would
build on a tip the network had already left. But a peer reports a *number*, not
a chain. On a fork that number is unreachable from where you stand — not
because you are behind, but because you are somewhere else. Both sides waited
for the other. Neither mined. Two branches, both frozen, every node politely
deferring.

The wait now expires after 45 seconds without progress. Our own tip moving is
progress and restarts the clock; if it stops while a peer still claims more,
the gap is a fork and waiting cannot close it. Mining on what turns out to be
the lighter branch wastes that miner's own electricity. A network where nobody
mines is broken for everybody.

**The wallet only ever added.**

Scanning collected outputs and marked them spent, and never did the reverse.
That is right while a chain only grows and wrong the moment it does not.

A coin received in a block that later lost a reorg stayed in the wallet as
spendable balance no node would accept. A send whose block was discarded still
read as confirmed. That is how the first real payment on this network vanished:
confirmed on the sender's chain, absent from the recipient's, both wallets
convinced they were right, and nothing anywhere saying otherwise.

Given a chain starting at genesis, absence now means absence. Outputs the chain
no longer holds are dropped, spent flags are recomputed rather than only set,
and a send whose inputs are no longer consumed returns to pending.
`unconfirmed_sends()` lists what needs resubmitting — nothing else can do it,
because block bodies are aggregated and a discarded block cannot be taken apart
into the transactions it held.

## Why a reset rather than an upgrade

The v6 chain ended with a transaction present on one branch and absent from
another, both branches stalled, and no honest way to say which was true. The
faults are fixed, but the history they produced cannot be repaired — only
picked. Restarting was cheaper than explaining, and this chain carried no value
to lose.

---

## Downloads

| File | Platform |
|------|----------|
| `NIGHTFALLCOIN-Core-0.5.0-macOS-arm64.dmg` | Apple Silicon, macOS 11+ |
| `NIGHTFALLCOIN-Core-0.5.0-macOS-intel.dmg` | Intel, macOS 10.15 Catalina+ |
| `nightfall-core-0.5.0-windows-x64.exe` | Windows 10+ — the wallet |
| `nightfalld-0.5.0-windows-x64.exe` | Windows — headless node |
| `nightfall-wallet-0.5.0-windows-x64.exe` | Windows — command-line wallet |

Verify against `SHA256SUMS-0.5.0.txt` and `SHA256SUMS-0.5.0-windows.txt`.

**Not code-signed.** macOS: right-click → **Open** → **Open**, once.
Windows: **More info → Run anyway**. The Windows binaries are built by
[a published workflow](https://github.com/Instinctes/nightfall/blob/main/.github/workflows/release.yml)
on GitHub's runners rather than on a laptop.

## First run

1. Open the wallet. It finds the network by itself.
2. **Settings → Backup.** Write the 24 words on paper *before* you mine anything. There is no reset and no support desk.
3. Press **Start mining**.

Mined coins show as *unlocking* for 1,440 blocks (~6 h).

## Honest status

Pre-launch software, not reviewed by anyone outside the project.

Every fault fixed in this release was found by real nodes disagreeing with each
other, not by the test suite — which passed throughout. That is the third time
today. The suite now covers both, but the pattern is the point: this software
has been correct in every test and wrong in the field more than once. Treat it
accordingly.

Do not put value on this that you cannot afford to lose entirely.

---

**Website:** https://nightfallcoin.org
