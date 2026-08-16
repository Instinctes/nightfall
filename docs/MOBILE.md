# Mobile wallet — architecture

**Scope:** a full iOS and Android wallet. Holds its own seed, receives, shows a
balance, and sends. Not a watch-only viewer.

**Status:** Android APK and an unsigned iOS IPA ship (sideload). Neither
is in a store. The browser wallet at `https://nightfallcoin.org/wallet/`
is the path that needs no Apple ID. Same `nightfall-wallet` code. See
`mobile/README.md`.

---

## 1. What the protocol gives us for free

Nightfall makes non-interactive one-sided payments. The sender constructs the
output alone, from the recipient's address, using an ECDH shared secret. The
recipient does nothing.

That is unusually good news for a phone:

- **Receiving works while the device is off.** No session, no handshake, no
  "both parties online" requirement that plain Mimblewimble imposes. A phone in
  a drawer still receives.
- **The address can be generated with no network at all.** Key generation is
  local arithmetic.
- **Nothing about receiving is time-sensitive**, so background execution limits
  on iOS — which are strict and not negotiable — cost us nothing.

The cryptography is also already portable. Every dependency in
`nightfall-crypto` is pure Rust with no C:

```
curve25519-dalek  bulletproofs  merlin  argon2
chacha20poly1305  ed25519-dalek x25519-dalek blake3  zeroize  subtle
```

All of these build for `aarch64-apple-ios` and `aarch64-linux-android`. **No
cryptography gets reimplemented in Swift or Kotlin.** That rule is not
stylistic. Wallets lose funds when a second implementation of key derivation
disagrees with the first in some edge case, and the disagreement is discovered
by a user whose coins are now unspendable.

---

## 2. What the protocol makes hard

### 2.1 Finding your own coins

There are no addresses on chain. To learn whether an output is yours, the
wallet computes an ECDH against its ephemeral key and compares the result to
the one-time key — see `scan_output` in `crates/nightfall-crypto/src/stealth.rs`.

That is **one scalar multiplication per output in the chain**. It is inherent to
stealth addressing; Monero has the identical cost. There is no index to look
something up in, because the absence of that index is the privacy property.

Rough numbers: ~0.15 ms per output on a desktop core, ~0.5 ms on a phone. One
million outputs is therefore around eight minutes of continuous computation on
mobile, and considerably more battery than anyone will tolerate.

**Mitigation — birth height.** A wallet created today cannot own an output from
before it existed. Recording the tip height at creation and scanning only
forward reduces a new mobile wallet's initial scan to zero. This is implemented
(`Wallet::birth_height`). It is not an optimisation so much as the difference
between usable and not.

Restoring an *old* seed from a mnemonic has no such shortcut and must scan from
the birth height the user supplies, or from genesis if they cannot remember.
The restore screen should ask for an approximate date and convert it to a
height, and should be explicit that a wrong guess means missing coins rather
than a wrong balance.

**Mitigation — view tags.** One byte derived from the shared secret, stored in
the output, lets a scanner discard a non-matching output after the ECDH but
before the second scalar multiplication. Measured, not estimated:

```text
per foreign output    before 61,590 ns    after 30,321 ns    2.03x
```

Reproduce with `cargo run --release -p nightfall-crypto --example scanbench`.

This was a consensus change to the output format and was made during the v6
reset, while the chain carried no value. Monero shipped the identical
construction in 2022 and needed a hard fork for it.

The remaining cost — one scalar multiplication per output — is irreducible. It
is what stealth addressing is.

### 2.2 The phone cannot validate the chain

Nighthash-v2 is Argon2id at 32 MiB per hash. Verification costs the same as one
mining attempt — that is the price of ASIC resistance, and it is paid by every
verifier including phones. A device that validated the chain would need 32 MiB
of memory traffic per block header, times the whole history, on a battery.

So the mobile wallet is a **trusting client**. It asks a node for chain data and
believes it. Consequences, stated plainly:

- A malicious node can **show a payment that does not exist**, or hide one that
  does.
- A malicious node can **lie about confirmation depth**, which matters if the
  user is accepting payment for something.
- A malicious node **cannot spend anything**. The seed never leaves the device.
  This is a display-integrity problem, not a theft problem.

**Mitigation:** default to the user's own node. The app ships pointed at
`seed.nightfallcoin.org` for convenience but the settings screen must make
"your own node" a first-class option, not an advanced one. Long-term, querying
two independent nodes and flagging disagreement is cheap and worth doing.

### 2.3 Sending reveals the origin

The node a transaction is submitted to is, with high probability, its origin
(stem/fluff starts *after* that hop). On a phone this is worse than on a
desktop because the device also carries a mobile IP that maps to a person.

The app must say this in plain language on the send screen, once, and not bury
it. Claiming privacy the implementation does not provide is the failure mode
that this project already documented once in `AUDIT-2026-08-12.md`.

---

## 3. Shape

```
┌──────────────┐  ┌──────────────┐  ┌────────────────────┐
│ iOS SwiftUI  │  │ Android      │  │ Browser PWA        │
│ (Xcode)      │  │ Compose      │  │ nightfallcoin.org  │
└──────┬───────┘  └──────┬───────┘  │ /wallet/           │
       │ uniffi          │ uniffi   └─────────┬──────────┘
       └────────┬────────┘                    │ wasm-bindgen
                ▼                             ▼
        nightfall-mobile              nightfall-web
                │                             │
                └──────────┬──────────────────┘
                           ▼
                   nightfall-wallet
                   scan, coins, spend — same as desktop
```

`nightfall-mobile` is a thin crate. Every rule it needs already exists in
`nightfall-wallet`, which the desktop GUI and the CLI also use. Adding a third
consumer must not fork that logic — if mobile needs behaviour the desktop does
not have, it goes in `nightfall-wallet` behind a flag, not in the FFI layer.

**Native UI per platform, not cross-platform.** A wallet is mostly system
integration: Keychain, Secure Enclave, biometrics, Android Keystore, background
limits, share sheets, camera for QR. Flutter and React Native make the shared
90 % easy and the remaining 10 % — which is the security-critical part — awkward.
The shared part here is Rust anyway.

---

## 4. Keys on the device

The seed is 32 bytes. Neither the Secure Enclave nor the Android StrongBox can
store arbitrary bytes — both hold *keys* they generate themselves. The standard
pattern applies:

**iOS**
1. Generate the seed in Rust, inside `zeroize`-guarded memory.
2. Create a P-256 key in the Secure Enclave with
   `kSecAccessControlBiometryCurrentSet` — this invalidates the key if the user
   adds a fingerprint or face, which is exactly what you want.
3. Encrypt the seed to that key; store only the ciphertext in the Keychain with
   `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`.
4. Decrypt on use. Never write the plaintext seed to disk, `UserDefaults`, or a
   log line.

**Android**
1. Same generation.
2. AES-256-GCM key in the Keystore, `setUserAuthenticationRequired(true)`,
   StrongBox where the hardware has it, `setInvalidatedByBiometricEnrollment(true)`.
3. Ciphertext in an app-private file.

`ThisDeviceOnly` / no-backup is deliberate on both. A seed that syncs to iCloud
or Google Drive is a seed whose security is now the cloud account's password.
**The mnemonic is the backup.** That is the entire reason it exists.

The output database — commitments, values, blinding factors, memos — is
*almost* as sensitive as the seed: it reveals every amount the user holds and
can be encrypted with the same key. On desktop it is already written `0600`.

---

## 5. Backup

`BIP39Mnemonic` (in `nightfall-crypto`) turns the 32-byte seed into 24 English
words and back. The list is the standard BIP-39 English wordlist; the checksum
catches transcription errors before they become support requests.

The flow is non-negotiable in one respect: **the wallet is not usable until the
user has confirmed the words.** Show them, require re-entry of three at random,
then unlock. Every wallet that made this skippable has a support inbox full of
people who skipped it.

Screenshots should be blocked on that screen — `FLAG_SECURE` on Android, and on
iOS by hiding the words when `UIScreen.isCaptured` or on
`userDidTakeScreenshotNotification`. iOS cannot prevent the screenshot, only
notice it, so also say in text why a photo is a bad idea.

---

## 6. Talking to a node

The current RPC is line-delimited JSON over plain TCP, unauthenticated, and
refuses to bind to a non-loopback address unless `NF_ALLOW_PUBLIC_RPC=1`. That
refusal is correct and must not be relaxed for mobile. `mine_one` alone is
reason enough — a public unauthenticated endpoint that mines on request is a
denial-of-service button.

The mobile endpoint is therefore a **separate, narrow, public surface**:

| | RPC (existing) | Mobile endpoint |
|--|--|--|
| Bind | loopback | public, TLS |
| Auth | none | none needed — read-only + submit |
| Methods | all, incl. `mine_one` | `status`, `scan_feed`, `scan_subscribe`, `submit_tx`, `get_utxo_root` |
| Transport | line JSON / TCP | HTTPS |

Simplest correct deployment: run the node with RPC on loopback, put a reverse
proxy in front that terminates TLS and forwards **only** the four methods above.
A phone must never reach `mine_one`.

### `scan_feed`

Blocks are dominated by Bulletproofs — 672 bytes per output, and the scanner
does not look at them. `scan_feed` returns, per height range:

- new outputs as `(commit, ephemeral_pk, output_pk, payload, features)`
- the commitments spent in that range
- the range's tip height and hash

That is roughly 150 bytes per output instead of 850 — about 5–6× less mobile
data. The client asks for full ranges, never for specific commitments: asking
for one output by name tells the node which output is yours and throws away the
privacy that local scanning buys.

### `scan_subscribe`

The same page as `scan_feed`, but the TCP connection stays open. After the
first page the node writes another line every time its tip moves, and an empty
`heartbeat: true` page every 30 seconds if nothing happened. The phone scans
each page locally with the same view key it would use on a one-shot `scan_feed`.
Closing the socket is how it unsubscribes.

A phone that polls `scan_feed` every few seconds is a phone that is always
behind the next block and always burning radio. One long-lived subscribe is
how a payment that lands while the app is open shows up before the user looks
away.

The CLI exposes this as `nightfall-wallet follow`.

---

## 7. Sending

The pieces already exist in `nightfall-wallet`: `select_coins_at`,
`build_transfer`, and the `Spendable` conversion. The mobile path is:

1. Scan a QR or paste an `nf1…` address. Verify the checksum before showing the
   confirm screen, not after.
2. Select coins, excluding immature coinbase outputs. **Mined coins are locked
   for 1,440 blocks (~6 h).** If the user just sent themselves a block reward
   from the desktop miner, the phone will show it as unavailable, and the UI has
   to explain that rather than looking broken.
3. Build the transaction locally. Change goes back to the wallet's own address.
4. Submit to the node, record it as pending.
5. Mark it confirmed when a block consumes the input commitments — the same
   mechanism the desktop uses, since a transaction does not survive aggregation
   as an identifiable object.

Two things the UI must get right because the protocol is unforgiving:

- **There is no recovery from a wrong address.** No refunds, no support desk,
  no reversal. Confirm screens should show the full address, not a truncation.
- **Fees are burned, not paid to a miner.** Worth one line in the UI, because
  users transplanted from other chains will assume a higher fee buys priority
  from someone who is being paid.

---

## 8. Order of work

| | | Depends on |
|--|--|--|
| 1 | Birth height | — *(done)* |
| 2 | BIP-39 mnemonic | — *(done)* |
| 3 | `scan_feed` RPC | — *(done)* |
| 3b | `scan_subscribe` stream | 3 *(done)* |
| 4 | View tags in the output format | — *(done, shipped in v6, carried into v7)* |
| 5 | `nightfall-mobile` uniffi crate | 1–4 | **done** |
| 6 | Public HTTP mobile API (`--mobile-listen`) | 3 | **done** (TLS = reverse proxy) |
| 7 | Android app | 5, 6 | **sources in `mobile/android`** |
| 8 | iOS app (sideload / EU, no App Store) | 5, 6 | **sources in `mobile/ios`** |

Android first, deliberately: no review gate, no developer-programme fee, and
sideloading a build onto a real device takes minutes. Every protocol-level
surprise gets found there, cheaply, before the same bug costs an App Store
review cycle.

---

## 9. Things that will go wrong

- **App Store review.** Apple applies extra scrutiny to wallets and has
  historically rejected non-custodial wallets for unclear reasons. Budget for
  rejection and appeal. Google is easier but requires a financial-features
  declaration.
- **Background scanning.** iOS will not let the app scan continuously. The
  balance is stale until the app is opened, and the UI must show "last synced
  at" rather than implying it is live.
- **Battery.** Scalar multiplication in a loop is exactly the shape of workload
  that gets an app flagged in the battery report. Scan in bounded chunks, only
  in the foreground, and stop at the tip.
- **Restore with no birth height.** Someone will restore a two-year-old seed and
  wait forty minutes. Show progress in blocks *and* an honest time estimate.
- **A second implementation.** The moment someone writes a Nightfall wallet in
  another language, key derivation has two implementations that must agree
  forever. Publish test vectors — address derivation, output scanning, mnemonic
  round-trip — before that happens rather than after.
