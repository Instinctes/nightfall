
# Nightfall Pay: safe WASM payment requests

The browser can use the shared Rust payment-request parser.
Exported functions: `buildPaymentRequest` and `previewPaymentRequest`.
Amounts and expiry values are canonical decimal strings, not JS floats.
`previewPaymentRequest` takes a whole Unix timestamp in seconds.
The response is JSON text: never place any field into `innerHTML`.
Show destination, network, amount, fee and expiry to the user.
A separate explicit approval is required before sending.
No keys are loaded, no transaction is prepared or broadcast.
Untrusted payment URIs must not be logged or sent to analytics.
An expired invoice is app policy, not a consensus restriction.
This is only the Rust/WASM boundary, not a finished browser UI.
Browser integration, real-device QA and independent audit remain.
Run `cargo test --locked -p nightfall-web pay_request_bridge`.
