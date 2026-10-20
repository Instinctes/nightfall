# Paradigm Creator: challenge-bound, operator-pinned node witnesses (experimental)

This **optional** layer signs the already merged `scripts/node-evidence.py` receipts
(PR #48) with Ed25519 operator keys. A verifier fixes a fresh 256-bit challenge
and an **out-of-band, explicitly pinned list of public-key fingerprints**.
At least two distinct, pinned keys must sign evidence for the same challenge.
No untrusted public key is automatically enrolled.

**What it adds:** evidence authorship and challenge-scoped replay resistance,
stronger than the existing unauthenticated SHA-256 receipt digest. Offline
comparisons detect incompatible reported public chain states at equal heights.
The result records only the number of unique keys and block heights, not
operator addresses, wallet identities, private keys, payment details or endpoints.

**What it cannot prove:** a signer's honesty; distinct node operators; correct
node software; real-time freshness of the RPC observation; authentic status
from a remote node; correct proof-of-work, block signatures, UTXO state,
cryptographic supply integrity or finality. The operator signs a **reported
observation**, not a consensus proof. Two keys on one machine are still one
operator. Equal height with conflicting tips may be an ordinary transient fork,
not proof of misconduct. Never accept this result as a permission to transfer
or mine.

## Test before use (offline, Python 3 and OpenSSL 3)

```sh
python3 scripts/test-node-evidence.py
python3 scripts/test-operator-witness.py
```

## Exercise (disposable operator keys only)

1. The independent examiner generates a new challenge **before** capture:
   `openssl rand -hex 32`. Share the challenge with each operator by a trusted
   channel. Only lower-case 64-character hex is accepted.
2. Each operator records a **local, read-only** observation with the merged
   `node-evidence.py`. Never expose the unauthenticated node RPC publicly:

   ```sh
   python3 scripts/node-evidence.py capture --output node.json
   openssl genpkey -algorithm ED25519 -out witness-private.pem
   chmod 600 witness-private.pem
   python3 scripts/operator-witness.py sign \
     --receipt node.json --private-key witness-private.pem \
     --challenge YOUR_64_HEX_CHALLENGE --output signed-node.json
   ```

3. **Before** trusting any submission, the examiner obtains each operator's
   Ed25519 SPKI SHA-256 fingerprint **out of band** and pins two or more in a
   local `trusted.json` file. The fingerprint is in `signed-node.json` under
   `body.operator` but copying only that self-declared value is *not* an
   independent verification of identity.

   ```json
   {"schema":"nightfall-operator-trust-v1","fingerprints":[
     "FIRST_VERIFIED_64_HEX_FINGERPRINT",
     "SECOND_VERIFIED_64_HEX_FINGERPRINT"
   ]}
   ```

   Then run:

   ```sh
   python3 scripts/operator-witness.py verify \
     --trust trusted.json --challenge YOUR_64_HEX_CHALLENGE \
     signed-node-A.json signed-node-B.json
   ```

Status 0: consistent, signed **reported** state; status 1: contradictions,
height disagreement or cautions; status 2: invalid input, signature, pin or
other operational error. The signer refuses to overwrite any output and
rejects private-key permissions broader than mode 600.

## Merge policy

- Additive and opt-in: no changes to consensus, daemon, wallet, mining,
  network behavior, existing receipts or their format.
- All unit tests run without live nodes, funded wallets or network access.
- Requires maintainer review and green CI before merging.
- A follow-up integration should only be considered once independent operators
  have independently confirmed threat models, key distribution and consent.

**Roadmap relation:** independent operator infrastructure, externally reviewable
evidence, and the independence checkpoint. **Whitepaper relation:** accurately
preserves the distinction between local verification and third-party RPC claims.
