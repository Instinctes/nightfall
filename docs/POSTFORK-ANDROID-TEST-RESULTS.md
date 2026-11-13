# NIGHTFALLCOIN: Postfork Regression Evidence

Basis: PR #10, Commit 7a356ca7c6ec160a1b11f23af154fd060bd42ff3

Plattform: Android aarch64 / Termux / Rust 1.99.1

Ergebnis: 5/5 Property-Tests, 1/1 echter TCP-Zwei-Node-Test.

Reproduktionsbefehle:

    cargo test --locked -p nightfall-node --lib post_activation_fork_choice_property_tests -- --test-threads=1
    cargo test --locked -p nightfall-node --test shorter_heavier -- --test-threads=1

SHA256 des lokalen Testlogs: 20d512d47fe01253e19e5933847f7623c735f47edf8338d4941142e800b58421

Keine Produktions-Konsensregeln geaendert.
Keine unabhaengige Mainnet-Konsensverifikation.
