# Nightfall Pay: community chat adapter

## Purpose

An offline, non-custodial adapter for the Pay links in chat
roadmap milestone.

Unlike the existing Discord administration scripts, this
adapter handles payment requests, not server configuration.

## Architecture

The existing Rust PaymentRequest implementation remains
the sole authority for payment syntax and validation.

The adapter adds platform-specific JSON message generation.

No new dependencies, wallet access, signing, RPC calls,
transaction submission or consensus changes are introduced.

## Security policy

- Explicit network selection.
- Canonical payment-link round trips.
- Exact integer amounts through the shared wallet parser.
- Visible recipient address and network.
- Mainnet real-funds warning.
- Rejection of expired and zero-amount requests.
- Rejection of nonempty memo and invoice fields.
- Disabled Discord mentions and API embeds.
- Disabled Telegram API link previews.
- Bounded standard input and message sizes.
- Errors do not echo supplied payment requests.

Expiry is application policy, not a consensus restriction.

A payment request does not authenticate its author or prove
that a payment occurred. Verify the recipient independently.

Chat services can read shared payment details.
URL fragments are not encrypted messages.

## Build

    cargo build --locked -p nightfall-wallet --example nightfall-pay-chat

## Test

    cargo test --locked -p nightfall-wallet --example nightfall-pay-chat

## Usage

    cargo run --locked -p nightfall-wallet --example nightfall-pay-chat -- discord devnet

Supply the payment request on standard input, then EOF.

Replace discord with telegram to generate a Telegram payload.

The adapter does not send the payload. An authorized transport
must provide the destination and appropriate API credentials.

No credentials are required to run the adapter itself.

## Scope and limitations

This is an integration foundation, not a deployed chat bot.

Real API integration, chat authorization, rate limiting,
operational monitoring and abuse controls remain future work.

Link preview settings apply to generated API payloads,
not manually pasted messages.

Local tests do not constitute an independent security audit.
