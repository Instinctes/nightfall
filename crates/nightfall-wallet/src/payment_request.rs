//! The payment request: what a payee asks for, in one line a payer can check.
//!
//! ```text
//! nightfall:nf1<address>?network=mainnet&amount=150000000&memo=Coffee&id=A-17
//! ```
//!
//! Four decisions carry the weight here, and each one is a refusal rather than
//! a convenience.
//!
//! **The network is required.** A Nightfall address carries no network tag:
//! `nf1…` is the same string on mainnet, testnet and devnet, and the same keys
//! answer for it everywhere. So a request written on testnet, pasted into a
//! mainnet wallet, would be paid with real coins — and nothing in the address
//! could have warned anyone. The network is therefore part of the request and
//! a missing one is an error, never an assumption.
//!
//! **The amount is in whole darks.** Decimal would mean parsing `0.1` on both
//! sides and agreeing about it, and every rounding disagreement between payer
//! and payee is money. An integer count of the smallest unit cannot round.
//!
//! **Unknown parameters are refused, not ignored.** A reader that skips a
//! parameter it does not know is a reader that will one day skip the one that
//! changed what was being asked for. If this format grows a field, old wallets
//! should say "I do not understand this request" rather than quietly pay an
//! older reading of it.
//!
//! **Nothing here signs anything.** A request is a statement of what someone
//! wants; parsing it produces a description for a human to confirm. The
//! decision to spend stays with the owner, in the wallet, after they have seen
//! the address and the amount.
//!
//! Expiry, if present, is *application policy*. The chain has no opinion about
//! it: a transaction is not invalid because an invoice says it is late, and a
//! wallet that treats an expiry as a consensus rule would be lying to its
//! owner. It is shown, and it is not enforced here.

use nightfall_crypto::Address;
use nightfall_types::{NetworkId, DARKS_PER_NIGHT, MAX_SUPPLY_DARKS};
use std::fmt;

/// The scheme. Lower-case here; parsing accepts any case, as schemes are
/// case-insensitive and a link that survived a spreadsheet may be shouting.
pub const SCHEME: &str = "nightfall";

/// Longest request accepted. A QR code cannot usefully hold more, and an
/// unbounded parser is a parser someone will feed a megabyte to.
pub const MAX_LEN: usize = 2048;
/// Longest memo. Matches the wallet's own memo limit.
pub const MAX_MEMO: usize = 64;
/// Longest invoice identifier. Long enough for a UUID and a prefix.
pub const MAX_INVOICE: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum RequestError {
    TooLong,
    NotARequest,
    BadAddress,
    MissingNetwork,
    UnknownNetwork(String),
    WrongNetwork { asked: NetworkId, wallet: NetworkId },
    BadAmount,
    AmountTooLarge,
    MemoTooLong,
    InvoiceTooLong,
    BadExpiry,
    UnknownParameter(String),
    RepeatedParameter(String),
    BadEncoding,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => write!(f, "This payment request is too long to be genuine."),
            Self::NotARequest => write!(
                f,
                "That is not a Nightfall payment request. It should begin with `{SCHEME}:`."
            ),
            Self::BadAddress => write!(
                f,
                "The address in this request is not a valid Nightfall address."
            ),
            Self::MissingNetwork => write!(
                f,
                "This request does not say which network it is for. A Nightfall \
                 address looks the same on every network, so without that it is \
                 impossible to tell real coins from test coins. Ask for a new request."
            ),
            Self::UnknownNetwork(n) => write!(
                f,
                "This request names a network this wallet does not know: {n}."
            ),
            Self::WrongNetwork { asked, wallet } => write!(
                f,
                "This request is for {asked} and this wallet is on {wallet}. \
                 Paying it here would send the wrong kind of coin to an address \
                 that would accept it.",
                asked = asked.as_str(),
                wallet = wallet.as_str()
            ),
            Self::BadAmount => write!(
                f,
                "The amount must be a whole number of darks, with no sign, \
                 decimal point or spaces."
            ),
            Self::AmountTooLarge => write!(
                f,
                "The amount is larger than every coin that will ever exist."
            ),
            Self::MemoTooLong => write!(f, "The memo is longer than {MAX_MEMO} characters."),
            Self::InvoiceTooLong => write!(
                f,
                "The invoice identifier is longer than {MAX_INVOICE} characters."
            ),
            Self::BadExpiry => write!(
                f,
                "The expiry must be a whole number of seconds since 1970."
            ),
            Self::UnknownParameter(p) => write!(
                f,
                "This request contains `{p}`, which this wallet does not understand. \
                 It may be asking for something newer than this version can honour, \
                 so nothing has been prepared. Update the wallet, or ask the payee \
                 for a plain request."
            ),
            Self::RepeatedParameter(p) => write!(
                f,
                "This request gives `{p}` more than once, so what it asks for is ambiguous."
            ),
            Self::BadEncoding => write!(
                f,
                "This request contains characters that cannot be decoded."
            ),
        }
    }
}

impl std::error::Error for RequestError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentRequest {
    pub address: Address,
    pub network: NetworkId,
    /// `None` means the payee did not name a figure and the payer chooses.
    pub amount_darks: Option<u64>,
    pub memo: String,
    /// The payee's own reference. Opaque here; it is their bookkeeping.
    pub invoice: String,
    /// Seconds since 1970, as *policy*. Never a consensus rule — see the
    /// module documentation.
    pub expires_unix: Option<u64>,
}

impl PaymentRequest {
    /// Parse a request, refusing anything ambiguous.
    ///
    /// This does not check the amount against a balance and does not prepare,
    /// sign or send anything. It answers one question: what is being asked for?
    pub fn parse(text: &str) -> Result<Self, RequestError> {
        let text = text.trim();
        if text.len() > MAX_LEN {
            return Err(RequestError::TooLong);
        }
        // Schemes are case-insensitive; nothing after them is.
        let rest = text
            .char_indices()
            .find(|(_, c)| *c == ':')
            .map(|(i, _)| (&text[..i], &text[i + 1..]))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(SCHEME))
            .map(|(_, rest)| rest)
            .ok_or(RequestError::NotARequest)?;

        let (address_part, query) = match rest.split_once('?') {
            Some((a, q)) => (a, q),
            None => (rest, ""),
        };
        let address = Address::decode(address_part.trim()).map_err(|_| RequestError::BadAddress)?;

        let mut network = None;
        let mut amount_darks = None;
        let mut memo = None;
        let mut invoice = None;
        let mut expires_unix = None;

        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, raw) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode_component(raw)?;
            // Repeats are refused rather than last-wins: two amounts in one
            // request means nobody knows which one the payee meant.
            match key {
                "network" => {
                    if network.is_some() {
                        return Err(RequestError::RepeatedParameter(key.into()));
                    }
                    network = Some(match value.as_str() {
                        "mainnet" => NetworkId::Mainnet,
                        "testnet" => NetworkId::Testnet,
                        "devnet" => NetworkId::Devnet,
                        other => return Err(RequestError::UnknownNetwork(other.to_owned())),
                    });
                }
                "amount" => {
                    if amount_darks.is_some() {
                        return Err(RequestError::RepeatedParameter(key.into()));
                    }
                    amount_darks = Some(parse_darks(&value)?);
                }
                "memo" => {
                    if memo.is_some() {
                        return Err(RequestError::RepeatedParameter(key.into()));
                    }
                    if value.chars().count() > MAX_MEMO {
                        return Err(RequestError::MemoTooLong);
                    }
                    memo = Some(value);
                }
                "id" => {
                    if invoice.is_some() {
                        return Err(RequestError::RepeatedParameter(key.into()));
                    }
                    if value.chars().count() > MAX_INVOICE {
                        return Err(RequestError::InvoiceTooLong);
                    }
                    invoice = Some(value);
                }
                "expires" => {
                    if expires_unix.is_some() {
                        return Err(RequestError::RepeatedParameter(key.into()));
                    }
                    expires_unix = Some(parse_u64(&value).ok_or(RequestError::BadExpiry)?);
                }
                other => return Err(RequestError::UnknownParameter(other.to_owned())),
            }
        }

        Ok(Self {
            address,
            network: network.ok_or(RequestError::MissingNetwork)?,
            amount_darks,
            memo: memo.unwrap_or_default(),
            invoice: invoice.unwrap_or_default(),
            expires_unix,
        })
    }

    /// Refuse a request meant for a different network.
    ///
    /// Separate from parsing on purpose: a wallet should be able to read a
    /// testnet request and *explain* why it will not pay it, rather than
    /// failing to understand it at all.
    pub fn require_network(&self, wallet: NetworkId) -> Result<(), RequestError> {
        if self.network == wallet {
            return Ok(());
        }
        Err(RequestError::WrongNetwork {
            asked: self.network,
            wallet,
        })
    }

    /// True when the payee's own deadline has passed.
    ///
    /// Policy, not consensus. A payment made after this is perfectly valid on
    /// the chain; whether the payee still honours it is between the two of
    /// them, and the wallet's job is to say so before the owner spends.
    pub fn is_expired(&self, now_unix: u64) -> bool {
        self.expires_unix
            .is_some_and(|deadline| now_unix > deadline)
    }

    /// The request as a link, in a form this parser accepts back.
    pub fn to_uri(&self) -> String {
        let mut uri = format!("{SCHEME}:{}", self.address.encode());
        let mut sep = '?';
        // Network first: it is the part a reader should see without scrolling.
        uri.push(sep);
        uri.push_str("network=");
        uri.push_str(self.network.as_str());
        sep = '&';
        if let Some(darks) = self.amount_darks {
            uri.push(sep);
            uri.push_str(&format!("amount={darks}"));
        }
        if !self.memo.is_empty() {
            uri.push(sep);
            uri.push_str(&format!("memo={}", encode_component(&self.memo)));
        }
        if !self.invoice.is_empty() {
            uri.push(sep);
            uri.push_str(&format!("id={}", encode_component(&self.invoice)));
        }
        if let Some(expires) = self.expires_unix {
            uri.push(sep);
            uri.push_str(&format!("expires={expires}"));
        }
        uri
    }

    /// The amount as a person reads it, or a plain statement that there is none.
    pub fn amount_text(&self) -> String {
        match self.amount_darks {
            Some(darks) => format!(
                "{}.{:08} NIGHT",
                darks / DARKS_PER_NIGHT,
                darks % DARKS_PER_NIGHT
            ),
            None => "any amount — you choose".to_owned(),
        }
    }
}

/// Whole darks only: digits, nothing else.
///
/// No sign, no decimal point, no underscores, no leading `+`. Rust's own
/// `u64::from_str` accepts a leading `+`, which would make `+5` and `5` two
/// spellings of one amount, and two spellings are one more than a payment
/// instruction should have.
fn parse_darks(value: &str) -> Result<u64, RequestError> {
    let darks = parse_u64(value).ok_or(RequestError::BadAmount)?;
    if darks > MAX_SUPPLY_DARKS {
        return Err(RequestError::AmountTooLarge);
    }
    Ok(darks)
}

fn parse_u64(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // `0` is fine; `007` is a second spelling of `7` and is not.
    if value.len() > 1 && value.starts_with('0') {
        return None;
    }
    value.parse().ok()
}

/// Percent-decoding, with `+` left alone.
///
/// `+` means a space in HTML form submissions and means a literal `+` in a
/// URI. This is not a form, so it is a literal — and a memo is displayed to a
/// human who should see what the payee typed.
fn decode_component(raw: &str) -> Result<String, RequestError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(RequestError::BadEncoding)?;
            let text = std::str::from_utf8(hex).map_err(|_| RequestError::BadEncoding)?;
            out.push(u8::from_str_radix(text, 16).map_err(|_| RequestError::BadEncoding)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| RequestError::BadEncoding)
}

/// Percent-encode everything that is not plainly safe.
///
/// Deliberately conservative: anything outside unreserved ASCII is escaped,
/// including characters a lenient encoder would leave alone. A request travels
/// through chat apps, spreadsheets and QR scanners, and each of them has its
/// own opinion about punctuation.
fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    fn address() -> Address {
        WalletKeys::from_seed([3; 32]).address()
    }

    fn uri(query: &str) -> String {
        format!("{SCHEME}:{}?{query}", address().encode())
    }

    #[test]
    fn an_ordinary_request_parses_and_survives_a_round_trip() {
        let request =
            PaymentRequest::parse(&uri("network=mainnet&amount=150000000&memo=Coffee&id=A-17"))
                .unwrap();
        assert_eq!(request.address, address());
        assert_eq!(request.network, NetworkId::Mainnet);
        assert_eq!(request.amount_darks, Some(150_000_000));
        assert_eq!(request.memo, "Coffee");
        assert_eq!(request.invoice, "A-17");
        assert_eq!(request.amount_text(), "1.50000000 NIGHT");
        assert_eq!(PaymentRequest::parse(&request.to_uri()).unwrap(), request);
    }

    /// The property the wallets' "build a request" buttons rest on.
    ///
    /// Both interfaces construct a `PaymentRequest`, call `to_uri`, and then
    /// read it back with this same parser, refusing to hand over anything that
    /// does not come back identical. That is deliberate: it means the memo
    /// bound, the amount bound and the encoding rules are enforced once, here,
    /// rather than restated on each screen where they could drift apart. This
    /// test is what makes the round trip safe to rely on — including for the
    /// awkward values a payee actually types.
    #[test]
    fn anything_this_builds_is_something_this_can_read_back() {
        let awkward = [
            "", "Coffee", "Rent — March", "a & b = c?", "100% of it", "kaffee für zwei",
            "  leading and trailing  ", "a+b", "50%25", "line\tbreak", "ünïcödé ✓",
            &"m".repeat(MAX_MEMO),
        ];
        for memo in awkward {
            for invoice in ["", "A-17", "3f2b8c10-0000-4aaa-bbbb-ccccccccdddd", "#7/2026"] {
                for amount in [None, Some(0), Some(1), Some(150_000_000), Some(MAX_SUPPLY_DARKS)] {
                    for expires in [None, Some(0), Some(1_800_000_000)] {
                        for network in
                            [NetworkId::Mainnet, NetworkId::Testnet, NetworkId::Devnet]
                        {
                            let request = PaymentRequest {
                                address: address(),
                                network,
                                amount_darks: amount,
                                memo: memo.trim().to_owned(),
                                invoice: invoice.to_owned(),
                                expires_unix: expires,
                            };
                            let uri = request.to_uri();
                            assert!(uri.len() <= MAX_LEN, "{uri}");
                            assert_eq!(
                                PaymentRequest::parse(&uri).as_ref(),
                                Ok(&request),
                                "did not survive: {uri}",
                            );
                        }
                    }
                }
            }
        }
    }

    /// …and the round trip refuses rather than truncates. A memo one character
    /// over the bound must fail to read back, because that refusal is the only
    /// thing standing between a payee and a request whose memo says something
    /// different from what they typed.
    #[test]
    fn an_over_long_memo_does_not_quietly_come_back_shorter() {
        let request = PaymentRequest {
            address: address(),
            network: NetworkId::Mainnet,
            amount_darks: None,
            memo: "m".repeat(MAX_MEMO + 1),
            invoice: String::new(),
            expires_unix: None,
        };
        assert_eq!(
            PaymentRequest::parse(&request.to_uri()),
            Err(RequestError::MemoTooLong),
        );
        let request = PaymentRequest {
            invoice: "i".repeat(MAX_INVOICE + 1),
            memo: String::new(),
            ..request
        };
        assert_eq!(
            PaymentRequest::parse(&request.to_uri()),
            Err(RequestError::InvoiceTooLong),
        );
    }

    /// The reason this format has a network field at all.
    ///
    /// `nf1…` is the same string on every network and the same keys answer for
    /// it, so a testnet request pasted into a mainnet wallet would be paid with
    /// real coins and the address could not have warned anyone.
    #[test]
    fn a_request_without_a_network_is_refused() {
        assert_eq!(
            PaymentRequest::parse(&uri("amount=1")),
            Err(RequestError::MissingNetwork),
        );
        let message = RequestError::MissingNetwork.to_string();
        assert!(
            message.contains("looks the same on every network"),
            "the refusal must explain why, got: {message}"
        );
    }

    #[test]
    fn a_request_for_another_network_is_read_but_not_payable() {
        // Read, so the wallet can explain — not rejected as gibberish.
        let request = PaymentRequest::parse(&uri("network=testnet&amount=1")).unwrap();
        assert_eq!(request.network, NetworkId::Testnet);
        assert_eq!(
            request.require_network(NetworkId::Mainnet),
            Err(RequestError::WrongNetwork {
                asked: NetworkId::Testnet,
                wallet: NetworkId::Mainnet,
            }),
        );
        request.require_network(NetworkId::Testnet).unwrap();
    }

    /// A reader that skips what it does not know will one day skip the field
    /// that changed what was being asked for.
    #[test]
    fn an_unknown_parameter_stops_everything() {
        assert_eq!(
            PaymentRequest::parse(&uri("network=mainnet&amount=1&surcharge=500")),
            Err(RequestError::UnknownParameter("surcharge".into())),
        );
    }

    #[test]
    fn a_repeated_parameter_is_ambiguous_not_last_wins() {
        for query in [
            "network=mainnet&amount=1&amount=999",
            "network=mainnet&network=testnet",
            "network=mainnet&memo=a&memo=b",
        ] {
            let error = PaymentRequest::parse(&uri(query)).unwrap_err();
            assert!(
                matches!(error, RequestError::RepeatedParameter(_)),
                "{query} should be refused as ambiguous, got {error:?}"
            );
        }
    }

    /// One amount, one spelling. `+5`, `05` and `5.0` must not all mean five.
    #[test]
    fn the_amount_is_whole_darks_with_exactly_one_spelling() {
        for bad in ["+5", "-5", "05", "5.0", "1e8", "0x10", "", "1_000", "١٢٣"] {
            assert_eq!(
                PaymentRequest::parse(&uri(&format!("network=mainnet&amount={bad}"))),
                Err(RequestError::BadAmount),
                "amount {bad:?} must be refused",
            );
        }
        // Whitespace inside the value, where the outer trim cannot reach it.
        // (A space at the very end of the line is a paste artefact and is
        // trimmed on purpose — see the pasted-request test.)
        for bad in ["5 ", " 5", "5%20"] {
            assert_eq!(
                PaymentRequest::parse(&uri(&format!("network=mainnet&amount={bad}&memo=x"))),
                Err(RequestError::BadAmount),
                "amount {bad:?} must be refused",
            );
        }
        // …while the honest spellings work, zero included.
        for good in ["0", "1", "150000000"] {
            let request =
                PaymentRequest::parse(&uri(&format!("network=mainnet&amount={good}"))).unwrap();
            assert_eq!(request.amount_darks, Some(good.parse().unwrap()));
        }
    }

    #[test]
    fn an_amount_beyond_every_coin_that_will_exist_is_refused() {
        let over = MAX_SUPPLY_DARKS + 1;
        assert_eq!(
            PaymentRequest::parse(&uri(&format!("network=mainnet&amount={over}"))),
            Err(RequestError::AmountTooLarge),
        );
        PaymentRequest::parse(&uri(&format!("network=mainnet&amount={MAX_SUPPLY_DARKS}"))).unwrap();
        // u64 overflow must read as a bad amount, not wrap into a small one.
        assert_eq!(
            PaymentRequest::parse(&uri("network=mainnet&amount=99999999999999999999999")),
            Err(RequestError::BadAmount),
        );
    }

    #[test]
    fn an_omitted_amount_means_the_payer_chooses() {
        let request = PaymentRequest::parse(&uri("network=mainnet")).unwrap();
        assert_eq!(request.amount_darks, None);
        assert_eq!(request.amount_text(), "any amount — you choose");
        assert_eq!(PaymentRequest::parse(&request.to_uri()).unwrap(), request);
    }

    #[test]
    fn memo_and_invoice_are_bounded_and_survive_punctuation() {
        let memo = "Tisch 7 — Kaffee & Kuchen, 50 %";
        let request = PaymentRequest::parse(&uri(&format!(
            "network=mainnet&memo={}",
            encode_component(memo)
        )))
        .unwrap();
        assert_eq!(request.memo, memo);
        assert_eq!(PaymentRequest::parse(&request.to_uri()).unwrap().memo, memo);

        let long = "x".repeat(MAX_MEMO + 1);
        assert_eq!(
            PaymentRequest::parse(&uri(&format!("network=mainnet&memo={long}"))),
            Err(RequestError::MemoTooLong),
        );
        assert_eq!(
            PaymentRequest::parse(&uri(&format!("network=mainnet&id={long}"))),
            Err(RequestError::InvoiceTooLong),
        );
    }

    /// `+` is a space in a form submission and a literal plus in a URI. This is
    /// not a form, and a memo is shown to a person who should see what was typed.
    #[test]
    fn a_plus_in_a_memo_stays_a_plus() {
        let request = PaymentRequest::parse(&uri("network=mainnet&memo=a+b")).unwrap();
        assert_eq!(request.memo, "a+b");
    }

    #[test]
    fn expiry_is_read_and_is_not_a_rule() {
        let request = PaymentRequest::parse(&uri("network=mainnet&expires=1800000000")).unwrap();
        assert_eq!(request.expires_unix, Some(1_800_000_000));
        assert!(!request.is_expired(1_799_999_999));
        assert!(
            !request.is_expired(1_800_000_000),
            "the deadline itself is not late"
        );
        assert!(request.is_expired(1_800_000_001));
        // A request with no deadline never expires, rather than expiring at zero.
        assert!(!PaymentRequest::parse(&uri("network=mainnet"))
            .unwrap()
            .is_expired(u64::MAX));
        assert_eq!(
            PaymentRequest::parse(&uri("network=mainnet&expires=soon")),
            Err(RequestError::BadExpiry),
        );
    }

    #[test]
    fn malformed_input_is_refused_before_anything_is_understood() {
        let valid = address().encode();
        assert_eq!(
            PaymentRequest::parse("bitcoin:1A1zP1?network=mainnet"),
            Err(RequestError::NotARequest),
        );
        assert_eq!(
            PaymentRequest::parse(&valid),
            Err(RequestError::NotARequest)
        );
        assert_eq!(
            PaymentRequest::parse(&format!("{SCHEME}:nf1deadbeef?network=mainnet")),
            Err(RequestError::BadAddress),
        );
        assert_eq!(
            PaymentRequest::parse(&format!("{SCHEME}:{}", "x".repeat(MAX_LEN))),
            Err(RequestError::TooLong),
        );
        assert_eq!(
            PaymentRequest::parse(&uri("network=mainnet&memo=%zz")),
            Err(RequestError::BadEncoding),
        );
        assert_eq!(
            PaymentRequest::parse(&uri("network=mainnet&memo=%e2%28%a1")),
            Err(RequestError::BadEncoding),
            "a byte sequence that is not UTF-8 must not become a lossy memo",
        );
        assert_eq!(
            PaymentRequest::parse(&uri("network=moonnet")),
            Err(RequestError::UnknownNetwork("moonnet".into())),
        );
    }

    /// Schemes are case-insensitive; a link that went through a spreadsheet may
    /// arrive shouting. Nothing after the scheme is case-insensitive.
    #[test]
    fn the_scheme_is_case_insensitive_and_the_rest_is_not() {
        let tail = format!("{}?network=mainnet", address().encode());
        PaymentRequest::parse(&format!("NIGHTFALL:{tail}")).unwrap();
        PaymentRequest::parse(&format!("NightFall:{tail}")).unwrap();
        assert_eq!(
            PaymentRequest::parse(&uri("NETWORK=mainnet")),
            Err(RequestError::UnknownParameter("NETWORK".into())),
        );
        assert_eq!(
            PaymentRequest::parse(&uri("network=MAINNET")),
            Err(RequestError::UnknownNetwork("MAINNET".into())),
        );
    }

    /// Surrounding whitespace is what a paste looks like, not an attack.
    #[test]
    fn a_pasted_request_with_whitespace_still_parses() {
        let padded = format!("  \n{}\t ", uri("network=mainnet&amount=1"));
        assert_eq!(
            PaymentRequest::parse(&padded).unwrap().amount_darks,
            Some(1)
        );
    }
}
