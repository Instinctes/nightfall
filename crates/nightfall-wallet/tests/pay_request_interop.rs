//! Portable Nightfall Pay conformance and adversarial testing.
//!
//! This executes the actual shared Rust parser.
//! It does not establish browser UI correctness, node security,
//! wallet safety, or the validity of financial transactions.
//!
//! The deterministic wallet address below is TEST ONLY.

use nightfall_crypto::WalletKeys;

use nightfall_types::{NetworkId, MAX_SUPPLY_DARKS};

use nightfall_wallet::payment_request::{PaymentRequest, RequestError, MAX_LEN, MAX_MEMO};

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    version: u8,
    scope: String,
    address_source: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    query: String,
    outcome: String,
    network: Option<String>,
    amount: Option<u64>,
    memo: Option<String>,
    invoice: Option<String>,
    expires: Option<u64>,
    error: Option<String>,
}

fn address() -> String {
    WalletKeys::from_seed([3u8; 32]).address().encode()
}

fn uri(query: &str) -> String {
    format!("nightfall:{}?{query}", address())
}

fn error_class(err: &RequestError) -> &'static str {
    match err {
        RequestError::TooLong => "TooLong",
        RequestError::NotARequest => "NotARequest",
        RequestError::BadAddress => "BadAddress",
        RequestError::MissingNetwork => "MissingNetwork",
        RequestError::UnknownNetwork(_) => "UnknownNetwork",
        RequestError::WrongNetwork { .. } => "WrongNetwork",
        RequestError::BadAmount => "BadAmount",
        RequestError::AmountTooLarge => "AmountTooLarge",
        RequestError::MemoTooLong => "MemoTooLong",
        RequestError::InvoiceTooLong => "InvoiceTooLong",
        RequestError::BadExpiry => "BadExpiry",
        RequestError::UnknownParameter(_) => "UnknownParameter",
        RequestError::RepeatedParameter(_) => "RepeatedParameter",
        RequestError::BadEncoding => "BadEncoding",
    }
}

fn network(value: &str) -> NetworkId {
    match value {
        "mainnet" => NetworkId::Mainnet,
        "testnet" => NetworkId::Testnet,
        "devnet" => NetworkId::Devnet,
        other => panic!("invalid fixture network: {other}"),
    }
}

#[test]
fn portable_contract_vectors_are_executed_against_real_parser() {
    let corpus: Corpus = serde_json::from_str(include_str!("fixtures/payment_request_v1.json"))
        .expect("portable vectors must be valid JSON");

    assert_eq!(corpus.version, 1);
    assert!(!corpus.scope.is_empty());

    assert!(
        corpus.address_source.contains("TEST-ONLY") || corpus.address_source.contains("Test-only")
    );

    assert!(corpus.cases.len() >= 30);

    let mut seen = std::collections::BTreeSet::new();

    for case in corpus.cases {
        assert!(
            seen.insert(case.id.clone()),
            "duplicate vector id {}",
            case.id
        );

        let parsed = PaymentRequest::parse(&uri(&case.query));

        match case.outcome.as_str() {
            "accept" => {
                assert!(
                    case.error.is_none(),
                    "{}: accepted vector has an error",
                    case.id
                );

                let request = parsed.unwrap_or_else(|e| panic!("{}: {e:?}", case.id));

                let n = network(case.network.as_deref().expect("missing fixture network"));

                assert_eq!(request.network, n, "{}", case.id);
                assert_eq!(request.amount_darks, case.amount, "{}", case.id);

                assert_eq!(request.memo, case.memo.unwrap_or_default(), "{}", case.id);

                assert_eq!(
                    request.invoice,
                    case.invoice.unwrap_or_default(),
                    "{}",
                    case.id
                );

                assert_eq!(request.expires_unix, case.expires, "{}", case.id);

                assert_eq!(
                    PaymentRequest::parse(&request.to_uri()),
                    Ok(request.clone())
                );

                // Reading a request never authorizes payment
                // on an incompatible network.
                for wallet_network in [NetworkId::Mainnet, NetworkId::Testnet, NetworkId::Devnet] {
                    assert_eq!(
                        request.require_network(wallet_network).is_ok(),
                        wallet_network == n
                    );
                }
            }

            "reject" => {
                assert!(
                    case.network.is_none() && case.amount.is_none(),
                    "{}: ambiguous reject vector",
                    case.id
                );

                let error = parsed.expect_err(&case.id);

                assert_eq!(
                    Some(error_class(&error)),
                    case.error.as_deref(),
                    "{}",
                    case.id
                );
            }

            other => panic!("unknown vector outcome: {other}"),
        }
    }
}

// Deterministic xorshift64*: no new dependencies,
// same workload and cases on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;

        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;

        self.0 = x;

        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
}

fn sample_text(rng: &mut Rng, cap: usize) -> String {
    let letters = [
        "a", "Z", "0", " ", "€", "文", "é", "✓", "🔒", "%", "&", "?", "=", "+", "/", "~",
    ];

    let length = rng.index(cap + 1);

    (0..length)
        .map(|_| letters[rng.index(letters.len())])
        .collect::<String>()
}

#[test]
fn deterministic_adversarial_roundtrips_and_parameter_injection() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);

    let address = WalletKeys::from_seed([3u8; 32]).address();

    let networks = [NetworkId::Mainnet, NetworkId::Testnet, NetworkId::Devnet];

    for trial in 0..3_072 {
        let request = PaymentRequest {
            address,

            network: networks[rng.index(networks.len())],

            amount_darks: Some(rng.next() % (MAX_SUPPLY_DARKS + 1)),

            memo: format!("x{}", sample_text(&mut rng, MAX_MEMO - 1)),

            invoice: format!("i{}", sample_text(&mut rng, 63)),

            expires_unix: Some(rng.next()),
        };

        let encoded = request.to_uri();

        assert!(encoded.len() <= MAX_LEN, "trial {trial}: oversized output");

        assert_eq!(
            PaymentRequest::parse(&encoded),
            Ok(request),
            "trial {trial}"
        );

        // Adversarial mutation of serialized requests.
        for key in ["network", "amount", "memo", "id", "expires"] {
            let attacked = format!("{encoded}&{key}=1");

            assert!(
                matches!(
                    PaymentRequest::parse(&attacked),
                    Err(RequestError::RepeatedParameter(ref p))
                        if p == key
                ),
                "trial {trial}: duplicate {key} was not rejected"
            );
        }

        let attacked = format!("{encoded}&new_payment_rule=1");

        assert!(matches!(
            PaymentRequest::parse(&attacked),
            Err(RequestError::UnknownParameter(_))
        ));
    }
}

#[test]
fn bounded_untrusted_input_never_panics_or_becomes_a_payable_default() {
    let mut rng = Rng(0x7AB5_9C32_AE09_1234);

    let alphabet = [
        "%", "&", "=", "?", "#", " ", "\n", "\t", "é", "🔒", "0", "1", "a", "Z", "!", "+",
    ];

    let prefix = format!("nightfall:{}?", address());

    for _ in 0..4_096 {
        let length = rng.index(240);
        let mut input = prefix.clone();

        for _ in 0..length {
            input.push_str(alphabet[rng.index(alphabet.len())]);
        }

        // Any panic is a test failure.
        let _ = PaymentRequest::parse(&input);
    }

    let oversized = format!("{}{}", uri("network=mainnet&memo="), "X".repeat(MAX_LEN));

    assert_eq!(
        PaymentRequest::parse(&oversized),
        Err(RequestError::TooLong)
    );
}
