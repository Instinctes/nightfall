use nightfall_types::NetworkId;
use nightfall_wallet::payment_request::PaymentRequest;
use serde_json::{json, Value};
use std::io::{self, Read};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_INPUT: usize = 8192;

#[derive(Clone, Copy)]
enum Platform {
    Discord,
    Telegram,
}

impl Platform {
    fn parse(text: &str) -> Result<Self, &'static str> {
        match text {
            "discord" => Ok(Self::Discord),
            "telegram" => Ok(Self::Telegram),
            _ => Err("Unsupported chat platform"),
        }
    }

    fn limit(self) -> usize {
        match self {
            Self::Discord => 2000,
            Self::Telegram => 4096,
        }
    }
}

fn network(text: &str) -> Result<NetworkId, &'static str> {
    match text {
        "mainnet" => Ok(NetworkId::Mainnet),
        "testnet" => Ok(NetworkId::Testnet),
        "devnet" => Ok(NetworkId::Devnet),
        _ => Err("Explicit network selection required"),
    }
}

fn render(
    platform: Platform,
    expected: NetworkId,
    input: &str,
    now: u64,
) -> Result<Value, &'static str> {
    if input.len() > MAX_INPUT {
        return Err("Request too large");
    }

    let request = PaymentRequest::parse(input).map_err(|_| "Invalid payment request")?;

    request
        .require_network(expected)
        .map_err(|_| "Wrong payment network")?;

    if request.is_expired(now) {
        return Err("Expired payment request");
    }

    if request.amount_darks == Some(0) {
        return Err("Zero-amount request");
    }

    if !request.memo.is_empty() || !request.invoice.is_empty() {
        return Err("Public chat must not contain private references");
    }

    let link = request.to_share_link();

    let roundtrip = PaymentRequest::parse(&link).map_err(|_| "Canonical link validation failed")?;

    if roundtrip != request {
        return Err("Canonical link changed request");
    }

    let label = match request.network {
        NetworkId::Mainnet => "MAINNET - real funds",
        NetworkId::Testnet => "TESTNET - test funds",
        NetworkId::Devnet => "DEVNET - test funds",
    };

    let expiry = match request.expires_unix {
        Some(value) => format!("Expires (Unix UTC): {value}"),
        None => "No expiry specified".to_owned(),
    };

    let lines = [
        "Nightfall payment request".to_owned(),
        format!("Network: {label}"),
        format!("Amount: {}", request.amount_text()),
        format!("Recipient: {}", request.address.encode()),
        expiry,
        "Verify the recipient independently.".to_owned(),
        "Review the amount and network in your wallet.".to_owned(),
        "This is not proof of payment or identity.".to_owned(),
        link,
    ];

    let text = lines.join("\n");

    if text.encode_utf16().count() > platform.limit() {
        return Err("Chat message exceeds platform limit");
    }

    match platform {
        Platform::Discord => Ok(json!({
            "content": text,
            "allowed_mentions": {"parse": []},
            "flags": 4
        })),
        Platform::Telegram => Ok(json!({
            "text": text,
            "link_preview_options": {
                "is_disabled": true
            }
        })),
    }
}

fn run() -> Result<(), &'static str> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 3 {
        return Err("Usage: nightfall-pay-chat \
             PLATFORM NETWORK < request.txt");
    }

    let platform = Platform::parse(&args[1])?;
    let expected = network(&args[2])?;

    let mut input = String::new();

    io::stdin()
        .take((MAX_INPUT + 1) as u64)
        .read_to_string(&mut input)
        .map_err(|_| "Unable to read request")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Invalid system clock")?
        .as_secs();

    let result = render(platform, expected, &input, now)?;

    let output =
        serde_json::to_string_pretty(&result).map_err(|_| "Unable to serialize payload")?;

    println!("{output}");

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    fn fixture(query: &str) -> String {
        // Public test fixture. Never fund this address.
        let address = WalletKeys::from_seed([3; 32]).address();
        format!("nightfall:{}?{query}", address.encode())
    }

    fn dev(platform: Platform, query: &str) -> Result<Value, &'static str> {
        render(platform, NetworkId::Devnet, &fixture(query), 100)
    }

    #[test]
    fn discord_transport_policy() {
        let result = dev(Platform::Discord, "network=devnet&amount=150000000").unwrap();

        assert_eq!(result["flags"], json!(4));
        assert_eq!(result["allowed_mentions"]["parse"], json!([]));

        let text = result["content"].as_str().unwrap();

        assert!(text.contains("1.50000000 NIGHT"));
        assert!(text.contains("DEVNET - test funds"));
        assert!(text.contains("Recipient: nf1"));
        assert!(text.contains("not proof of payment"));
    }

    #[test]
    fn telegram_transport_policy() {
        let result = dev(Platform::Telegram, "network=devnet&amount=1").unwrap();

        assert_eq!(result["link_preview_options"]["is_disabled"], json!(true));

        assert!(result.get("parse_mode").is_none());
        assert!(result.get("chat_id").is_none());

        assert!(result["text"]
            .as_str()
            .unwrap()
            .contains("0.00000001 NIGHT"));
    }

    #[test]
    fn unsafe_requests_fail_closed() {
        let queries = [
            "amount=1",
            "network=mainnet&amount=1",
            "network=testnet&amount=1",
            "network=devnet&amount=0",
            "network=devnet&amount=1&amount=2",
            "network=devnet&amount=1.5",
            "network=devnet&amount=01",
            "network=devnet&expires=99",
            "network=devnet&memo=private",
            "network=devnet&id=private",
            "network=devnet&memo=%40everyone",
            "network=devnet&memo=%0Aprivate",
            "network=devnet&future_field=1",
        ];

        for query in queries {
            for platform in [Platform::Discord, Platform::Telegram] {
                assert!(dev(platform, query).is_err(), "{query}");
            }
        }
    }

    #[test]
    fn open_amount_remains_open() {
        let result = dev(Platform::Telegram, "network=devnet").unwrap();

        assert!(result["text"].as_str().unwrap().contains("any amount"));
    }

    #[test]
    fn expiry_uses_original_parser_policy() {
        for expiry in [100, 101] {
            let query = format!("network=devnet&expires={expiry}");
            let result = dev(Platform::Discord, &query).unwrap();

            let expected = format!("Expires (Unix UTC): {expiry}");

            assert!(result["content"].as_str().unwrap().contains(&expected));
        }
    }

    #[test]
    fn canonical_output_roundtrips() {
        let original = fixture("network=devnet&amount=123");
        let parsed = PaymentRequest::parse(&original).unwrap();

        for platform in [Platform::Discord, Platform::Telegram] {
            let result = render(platform, NetworkId::Devnet, &original, 100).unwrap();

            let key = match platform {
                Platform::Discord => "content",
                Platform::Telegram => "text",
            };

            let text = result[key].as_str().unwrap();
            let link = text.lines().last().unwrap();

            assert_eq!(PaymentRequest::parse(link).unwrap(), parsed);

            assert!(link.starts_with("https://wallet.nightfallcoin.org/#pay="));

            assert!(text.contains(&parsed.address.encode()));
        }
    }

    #[test]
    fn explicit_mainnet_warning() {
        let result = render(
            Platform::Discord,
            NetworkId::Mainnet,
            &fixture("network=mainnet&amount=1"),
            100,
        )
        .unwrap();

        assert!(result["content"]
            .as_str()
            .unwrap()
            .contains("MAINNET - real funds"));
    }

    #[test]
    fn explicit_testnet_warning() {
        let result = render(
            Platform::Telegram,
            NetworkId::Testnet,
            &fixture("network=testnet&amount=1"),
            100,
        )
        .unwrap();

        assert!(result["text"]
            .as_str()
            .unwrap()
            .contains("TESTNET - test funds"));
    }

    #[test]
    fn hostile_inputs_are_refused() {
        for input in [
            String::new(),
            "not a request".to_owned(),
            "https://example.org/#pay=test".to_owned(),
            "x".repeat(MAX_INPUT + 1),
        ] {
            assert!(render(Platform::Discord, NetworkId::Devnet, &input, 100,).is_err());
        }
    }

    #[test]
    fn errors_do_not_echo_private_input() {
        let secret = "PRIVATE_REFERENCE_123";

        let query = format!("network=devnet&memo={secret}");

        let error = dev(Platform::Discord, &query).unwrap_err();

        assert!(!error.contains(secret));
    }

    #[test]
    fn unsupported_arguments_are_refused() {
        assert!(Platform::parse("unknown").is_err());
        assert!(network("unknown").is_err());
        assert!(network("").is_err());
    }
}
