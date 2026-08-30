# NIGHT ↔ BTC swap — attacks we ran, and the ones we did not

v0.9.0. This is the honesty sheet for the warning in the wallet.

There is **no external cryptographic review** of the Ristretto DLEQ leaf
in `nightfall_crypto::dleq`. Tests below are ours. That is weaker than a
person who is trying to break it. Mainnet stays gated for that reason.

## What we forced, in this tree

| Attack / fault | Where | Result |
|---|---|---|
| Rogue key `S_b = P − S_a` | `swap::a_rogue_key_lets_one_party_spend_alone` | Theft without DLEQ. Production path is `from_verified_offers`. `SharedLock::new` is kept so the theft stays demonstrable. |
| DLEQ for statement X used as Y | `dleq::binding_tests`, `hardening_tests` | Refused |
| Mangled proof / garbage core | same | Fail closed |
| Seeded prove is deterministic | `a_seeded_prover_is_deterministic` | Holds |
| Lock to someone else's address | `verify_lock` | `NotOurOutput` |
| Wrong amount / view tag / payload | `verify_lock` | Distinct errors |
| Signed lying payload | `hostile_payload_tests` | `BadPayload`; still spendable from `t` |
| Alice redeems too close to H₁ | state machine, driver, live Lauf 4 | Wallet refuses; **the Bitcoin node would accept** |
| Pending redeem re-offered after H₁ | `a_block_arriving_between_ticks_withdraws_the_redeem` | Withdrawn, abort |
| Redeem while lock depth unknown | `a_pending_redeem_is_withheld_while_the_depth_is_unknown`, `a_redeem_is_not_intended_when_the_lock_depth_is_unknown` | No send |
| Outage as “0 confirmations” | driver H4, RPC pruned miss | Error / unknown, not zero |
| Crash while Redeeming → MustCancel | state resume | Stays Redeeming |
| World-readable secret file | persist | Refused |
| Share ≥ 2^252 on load | `SwapShare::from_bytes` | Refused |
| Packet id / net / seq / amount / checksum | packet tests | Refused |
| Live BIP68: cancel before H₁, punish before H₂ | `swap_live_aborts` against bitcoind v30.1 | Node rejects `non-BIP68-final` |
| Recover `s_a` from a confirmed witness | `swap_live` | Equals Alice's share |

## What we did not run

- Signet with `cancel = 144` (~a day of real blocks). The timing class is
  simulated by moving the fake chain between ticks, and by the soak loop.
- Weeks of unattended mainnet-adjacent operation. `tests/soak.rs` is the
  same invariant, hundreds of ticks, not weeks.
- A third-party review of the 252-bit Ristretto leaf.
- A hostile counterparty on a public network.
- Fee-ladder pre-signed rungs (handshake still signs one fee).
- Taproot, a mailbox, a NIGHT refund — all refused by construction.

## Known wart (inherited)

If Alice locks NIGHT and Bob cancels then never refunds, Alice punishes at
H₂ and **the NIGHT stays locked forever**. On screen. Not only in this file.

## §9.2, in one paragraph

Alice completing TX_redeem publishes `s_a`. If that transaction does not
confirm before H₁, Bob cancels, extracts `s_a` from the mempool, claims
NIGHT, refunds BTC. The driver must re-read Bitcoin depth every tick and
must not re-broadcast a pending redeem once the margin is gone. That is
not optional.
