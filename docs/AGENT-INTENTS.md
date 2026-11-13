# Owner-gated agent payment intentions (v1)

**Status:** opt-in CLI feature. Review-gated requests; NOT unattended agent accounts
and NOT cryptographic/on-chain delegation. No consensus, RPC, protocol, genesis,
fee-burn, issuance, web-wallet or wallet key changes.

## Why

The locked product attributes mention *agent-ready, capability-limited* account
types. An autonomous model must never receive a full Nightfall seed merely so
it can prepare a payment. The first safe, executable boundary is:

1. An untrusted agent creates a small structured payment **intention**.
2. An owner controls a private per-recipient, per-transaction **policy**.
3. The wallet checks the exact network, owner address, agent identifier,
   destination, amount, fee, timestamps, short ASCII memo and nonce.
4. **The owner** verifies the complete details at a local, interactive terminal
   and explicitly approves with the nonce.
5. The normal Nightfall wallet creates, submits and records the transaction.

No code path grants the agent signing authority. A rejection does not create
a transaction. This is an executable trust boundary, not a complete autonomous
wallet. The separate roadmap document at nightfallcoin.org/roadmap/ places
reliability and external review before wider product deployment.

## Example on disposable Devnet funds

Use the existing CLI wallet on a machine that the agent **cannot** access.

An owner creates a local policy JSON file, on a private filesystem, containing:

~~~json
{
  "format": "nightfall-agent-intent-v1",
  "network": "devnet",
  "wallet": "REPLACE_WITH_OWNER_NF1_ADDRESS",
  "agent_id": "invoice-agent",
  "recipient": "REPLACE_WITH_ALLOWED_RECIPIENT_NF1_ADDRESS",
  "max_amount_darks": 100000000,
  "max_fee_darks": 100000,
  "expires_at": 1790000000
}
~~~

Use a genuine future Unix expiry; the number above is **illustrative** and may
already be expired when you read this. On Unix, lock down this file:

~~~sh
chmod 600 owner-policy.json
~~~

The agent may create an unsigned JSON file such as:

~~~json
{
  "format": "nightfall-agent-intent-v1",
  "network": "devnet",
  "wallet": "REPLACE_WITH_OWNER_NF1_ADDRESS",
  "agent_id": "invoice-agent",
  "recipient": "REPLACE_WITH_ALLOWED_RECIPIENT_NF1_ADDRESS",
  "amount_darks": 10000000,
  "fee_darks": 10000,
  "memo": "Invoice 17",
  "nonce": "c4a68d4cb681c6c94e1adc4d9c83e6e2",
  "created_at": 1790000000,
  "expires_at": 1790000120
}
~~~

The agent must use a fresh unpredictable 16-byte nonce and timestamps that
are current at the moment of the request; the example values are not current.
The owner checks the full request and submits it from an interactive terminal:

~~~sh
nightfall-wallet --network devnet --seed-file owner.seed \
  agent-send --policy owner-policy.json --intent request.json
~~~

Approval requires typing the exact full command displayed by the wallet:
an 'APPROVE ' prefix followed by the 32-character request nonce.
Everything else rejects without signing.

The owner may also decline by pressing Enter. There is no auto-approval mode.

## Security limits — important

- **Per request, not per lifetime:** the caps do NOT bound aggregate spending.
  There is no replay journal, global allowance, delayed settlement or revoke
  registry. The nonce makes approval specific to a visible request, but a
  sufficiently privileged agent could resubmit the same request for another
  human approval. Never describe this as replay protection.
- **Trusted execution boundary:** the agent must have no ability to access the
  wallet seed, execute the normal CLI send command with it, change the policy,
  control the owner terminal, or replace the wallet binary. A process running
  with the owner's filesystem and terminal privileges bypasses this guard.
  A pseudo-terminal is not hardware human-presence attestation.
- **Policy file security:** restrictive Unix mode is required; the user is
  responsible for filesystem ownership and trusted operating system permissions.
  Symlink and nonregular input paths are rejected. Access-controlled deployment
  still needs a threat-model review.
- **Time:** the machine's system clock is trusted; no blockchain-confirmed time
  is claimed. An intention lasts at most 5 minutes from creation and cannot
  outlive the owner's policy. Existing policy files are not signatures.
- **JSON:** fixed fields only, max 4096 bytes, duplicate/extra fields rejected.
  Amounts and fees are unsigned integer darks, never floats. Agent-controlled
  memos are printable ASCII at most 64 bytes to prevent prompt/terminal spoofing.
  This is a narrow request format, not an arbitrary command execution API.
- **Privacy:** no service, telemetry, automated agent discovery or remote API
  is added. Sharing request files discloses the destination and amount to the
  agent, which inherently knows what it requested. Only the owner wallet signs.
- **Delivery:** local request checks do not prove sufficient funds, prevent a
  reorg, guarantee inclusion or prove eventual settlement. The existing
  wallet and node handle construction and transmission. Submission is not
  confirmation.

## Validation

~~~sh
cargo test -p nightfall-wallet agent_policy
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
~~~

Unit tests cover allowed requests, wrong network, wallet, agent and recipient,
zero/overflow/excess amount and fee, stale/future/too-long timestamps, terminal
spoofing and invalid nonces, duplicate and unknown JSON fields.

## Migration path

A future autonomous (unattended) signing service would need **independent
key custody, authenticated requests, a durable atomic reservation journal,
revocation and crash/reorg recovery**. Those are NOT implemented here. They
must be reviewed separately before anybody advertises agent-safe spending.

This change makes a valuable subset usable today without introducing a new
consensus rule, pretending policy checks are cryptography or exposing private
keys to agents.
