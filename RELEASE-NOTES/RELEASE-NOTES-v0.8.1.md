**Same chain.** Genesis `061a052d…`, protocol v8, wire v6. Nothing in this
release touches consensus. 0.8.0 and 0.7.x peer with it normally.

A wallet release, prompted by a payment that sat at "pending" for a day and a
half while the wallet said nothing useful about why.

## The Activity list stopped drifting sideways

Scrolling the Activity page pushed the amounts off to the right. The cause was
a row that asked `ui.available_width()` for its own budget and subtracted a
constant. That number changes the moment the scroll bar appears, so each row
measured a slightly different width and the right-aligned column walked across
the screen as you scrolled.

Rows are now laid out in fixed columns — icon, description, amount — that a row
cannot renegotiate. The scroll bar is reserved permanently rather than
appearing on overflow, so nothing else on any page shifts when a list gets long
enough to scroll. Long memos truncate instead of wrapping and shoving the rest
of the row around.

## The wallet now says why a payment is stuck

This is the part that matters.

A new transaction is handed to **exactly one** randomly chosen peer, so it
cannot be traced back to the node that created it. That is deliberate. What was
missing is that **nothing re-sends it.** If that one peer drops the
transaction — it restarts, the connection dies, it was a Tor exit that
vanished — the payment is gone and no other node ever hears about it.

The wallet knew this and told you "pending", forever, with no way to find out
what that meant short of reading the source.

Three changes:

- **The send confirmation no longer says "Sent".** It says *Submitted —
  waiting for a block*, because handing a transaction to your own node is not
  the same as it being in the chain, and that one word was worth hours of
  confusion.
- **If you send with fewer than three peers connected, the wallet warns you
  immediately** — while you are still looking at the screen and can do
  something about it. Few peers is exactly the condition where the single hop
  is also the only hop.
- **Activity shows a notice** when a send has been waiting more than half an
  hour: what happened, that the coins were never actually spent, and what to do
  about it.

**If you are holding a stuck payment right now:** your coins are safe. They
were never spent — the wallet has simply reserved them for a payment that died.
**Settings → Rescan from genesis** re-derives what is spent from the chain
itself and releases them. Then send again.

Automatic rebroadcast of unconfirmed sends is the real fix and is next.

## Downloads

`NIGHTFALLCOIN-Core-0.8.1-macOS-arm64.dmg`
`NIGHTFALLCOIN-Core-0.8.1-macOS-intel.dmg`
`nightfall-core-0.8.1-windows-x64.exe`
`nightfalld-0.8.1-windows-x64.exe`
`nightfall-wallet-0.8.1-windows-x64.exe`
`nightfall-core-0.8.1-linux-x64`
`nightfalld-0.8.1-linux-x64`
`nightfall-wallet-0.8.1-linux-x64`

Verify `SHA256SUMS-0.8.1.txt`, `SHA256SUMS-0.8.1-windows.txt`,
`SHA256SUMS-0.8.1-linux.txt`.

Phone and browser wallets stay 0.7.0. 210 tests, fmt and clippy clean.
