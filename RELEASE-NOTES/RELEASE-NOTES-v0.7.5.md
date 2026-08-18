**Same chain.** Genesis `061a052d…`, protocol v8, wire v6.

0.7.4 read every stored block as if a stranger had sent it: range proofs,
signatures, the supply equation, a ledger clone per height. Eleven
thousand blocks took about ten minutes. The window opened immediately
and looked empty — PEERS=0, supply 0, “mining alone” — while the file
was still being replayed. Miners thought the wallet was broken.

On a file this node already validated, restart now only checks that
each block still links to the last, rebuilds the UTXO set, and proves
the supply once. 12 000 blocks: a third of a second. P2P opens on the
real tip. A file that does not match `chain-meta.json` still gets the
full re-check, in the background.

## Downloads

`NIGHTFALLCOIN-Core-0.7.5-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.7.5-macOS-intel.dmg`
`nightfall-core-0.7.5-windows-x64.exe`
`nightfalld-0.7.5-windows-x64.exe`
`nightfall-wallet-0.7.5-windows-x64.exe`
`nightfall-core-0.7.5-linux-x64`
`nightfalld-0.7.5-linux-x64`
`nightfall-wallet-0.7.5-linux-x64`

Verify `SHA256SUMS-0.7.5.txt`, `SHA256SUMS-0.7.5-windows.txt`,
`SHA256SUMS-0.7.5-linux.txt`. Phone wallets stay 0.7.0.
