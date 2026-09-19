//! Air: paying from a wallet that is never on a network.
//!
//! # The split, and why it is this way round
//!
//! A Mimblewimble transaction cannot be built without the spender's outputs
//! and their blinding factors, and those come from the seed. So there is no
//! useful "watch-only signs nothing, cold device signs everything" split of the
//! kind a Bitcoin wallet has: **the cold side holds the wallet.** What the
//! online side holds is the node.
//!
//! Two packages cross the gap, and nothing else ever does:
//!
//! 1. An [`Intent`], online → cold. What to pay, to whom, at what fee, at what
//!    chain height, on which network. It carries no secret, so a photograph of
//!    it on a café table costs nothing.
//! 2. A [`Signed`] package, cold → online. The finished transaction, to be
//!    handed to the network. Opaque here: this module moves it and proves it
//!    arrived whole, and does not pretend to understand it.
//!
//! The seed never leaves the cold device, and the cold device never learns
//! anything about the network beyond what one line of text told it.
//!
//! # What the signer checks for itself
//!
//! The cold side must not sign a summary somebody else wrote. It parses the
//! intent into an [`Address`], a whole number of darks and a fee, shows *those*
//! to its owner, and then builds the transaction from the same parsed values.
//! There is no path where the string that is displayed and the values that are
//! signed come from different places.
//!
//! # Replay, and the two ways it goes wrong
//!
//! Every intent carries a random nonce and a network.
//!
//! - A [`Signed`] package quotes the nonce of the intent it answers, so a
//!   transaction cannot be presented as the answer to a different request.
//! - The online side records the nonces it has broadcast ([`NonceLog`]). A
//!   signed package that arrives twice — a QR left on a screen, a photograph
//!   taken again — is refused the second time rather than broadcast again.
//! - The cold side records the nonces it has signed, so one intent cannot be
//!   turned into two different transactions spending the same coins.
//! - The network is checked on both sides. A testnet intent signed by a
//!   mainnet wallet would spend real coins, and an address cannot warn anyone:
//!   `nf1…` is one string on every network.
//!
//! An intent also carries an expiry, because a fee that was right at one chain
//! height is not necessarily right a week later.
//!
//! # Transport
//!
//! [`frames`] cuts a payload into numbered pieces small enough to photograph,
//! each carrying the digest of the whole. [`Reassembler`] accepts them in any
//! order, ignores repeats, refuses pieces from a different transfer, and
//! checks the digest before handing anything back. A partial scan is a partial
//! scan; it never becomes a short payload.

use nightfall_crypto::{hash_multi, Address};
use nightfall_types::{NetworkId, MAX_SUPPLY_DARKS};
use std::collections::BTreeMap;
use std::fmt;

/// The scheme every Air package starts with.
pub const SCHEME: &str = "nightfall-air";
/// Format version. A reader that does not know a version refuses it whole.
pub const VERSION: u32 = 1;

/// Longest payload Air will carry, in bytes. Above this the transfer is not a
/// transfer, it is someone feeding a parser.
///
/// The number comes from measuring rather than from taste. A real Nightfall
/// payment — one input, two outputs, with its range proofs — serialises to
/// about eight kilobytes, and the package text carries it as hex, so what the
/// frames actually move is about sixteen. The first version of this module set
/// the budget at 12 800 and the end-to-end test refused the first real
/// transaction it was given.
///
/// The hex appears twice — once inside the package text, once in each frame —
/// which costs four bytes of transfer per byte of transaction. That is the
/// price of one inspectable text form that a person can paste into a file,
/// read, and compare; at these sizes it is around thirty frames, which is a
/// few seconds of animation, so it is worth paying.
pub const MAX_PAYLOAD: usize = 32_000;
/// Bytes of payload per frame. Chosen so the resulting QR code is comfortably
/// readable by a phone at arm's length rather than technically decodable: at
/// 500 bytes a frame is about a version 20 code, which a phone reads without
/// being held still.
pub const MAX_FRAME_PAYLOAD: usize = 500;
/// Most frames one transfer may have. `MAX_PAYLOAD / MAX_FRAME_PAYLOAD`.
pub const MAX_FRAMES: usize = 64;
/// Longest single frame string a reader will look at.
///
/// `MAX_FRAME_PAYLOAD` bytes become twice as many hex characters, plus the
/// header — so this has to be comfortably more than double the payload bound,
/// and when the payload bound moved this did not, which made every frame
/// unreadable at once.
pub const MAX_FRAME_LEN: usize = 2 * MAX_FRAME_PAYLOAD + 128;
/// Nonces remembered against replay. Old ones fall off the front.
pub const NONCE_MEMORY: usize = 512;

#[derive(Debug, PartialEq, Eq)]
pub enum AirError {
    NotAnAirPackage,
    UnknownVersion(u32),
    UnknownKind(String),
    MissingField(&'static str),
    UnknownField(String),
    RepeatedField(String),
    BadAddress,
    BadNumber(&'static str),
    AmountTooLarge,
    WrongNetwork { package: NetworkId, wallet: NetworkId },
    Expired { expired_at: u64, now: u64 },
    ReplayedNonce,
    NonceMismatch,
    PayloadTooLarge(usize),
    PayloadEmpty,
    BadFrame,
    FrameFromAnotherTransfer,
    TooManyFrames(usize),
    IncompleteTransfer { have: usize, total: usize },
    DigestMismatch,
}

impl fmt::Display for AirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnAirPackage => write!(
                f,
                "That is not an offline payment package. It should begin with `{SCHEME}:`."
            ),
            Self::UnknownVersion(v) => write!(
                f,
                "This package is version {v}, which this wallet does not know. A newer \
                 version may mean something different by the same words, so it is \
                 refused rather than half-understood."
            ),
            Self::UnknownKind(k) => write!(f, "This package says it is a `{k}`, which this wallet does not handle."),
            Self::MissingField(field) => write!(
                f,
                "This package does not say `{field}`, and that is not something a \
                 wallet may assume. Ask for a new one."
            ),
            Self::UnknownField(field) => write!(
                f,
                "This package contains `{field}`, which this wallet does not \
                 understand. It may be asking for something newer than this version \
                 can honour, so nothing was prepared."
            ),
            Self::RepeatedField(field) => write!(
                f,
                "This package gives `{field}` more than once, so what it asks for is ambiguous."
            ),
            Self::BadAddress => write!(f, "The address in this package is not a valid Nightfall address."),
            Self::BadNumber(field) => write!(
                f,
                "`{field}` must be a whole number, with no sign, decimal point or spaces."
            ),
            Self::AmountTooLarge => write!(f, "That is more NIGHT than will ever exist."),
            Self::WrongNetwork { package, wallet } => write!(
                f,
                "This package is for {} and this wallet is on {}. Signing it here \
                 would spend the wrong kind of coin, and the address cannot warn \
                 you — `nf1…` is the same string on every network.",
                package.as_str(),
                wallet.as_str(),
            ),
            Self::Expired { expired_at, now } => write!(
                f,
                "This request expired at {expired_at} and it is now {now}. The fee it \
                 names was chosen for a chain height that has passed. Ask for a new one."
            ),
            Self::ReplayedNonce => write!(
                f,
                "This package has been used already. Broadcasting it again would not \
                 send a second payment — it would re-send the same one — but a wallet \
                 that accepted it twice could not tell you which. Nothing was done."
            ),
            Self::NonceMismatch => write!(
                f,
                "This signed transaction answers a different request than the one it \
                 was matched with. Nothing was broadcast."
            ),
            Self::PayloadTooLarge(n) => write!(f, "This package is {n} bytes, more than Air carries."),
            Self::PayloadEmpty => write!(f, "There is nothing in this package."),
            Self::BadFrame => write!(f, "That is not a readable frame."),
            Self::FrameFromAnotherTransfer => write!(
                f,
                "That frame belongs to a different transfer. Scanning two at once \
                 would splice them together, so it was refused."
            ),
            Self::TooManyFrames(n) => write!(f, "A transfer of {n} frames is more than Air carries."),
            Self::IncompleteTransfer { have, total } => {
                write!(f, "{have} of {total} frames so far.")
            }
            Self::DigestMismatch => write!(
                f,
                "The frames are all here but do not add up to what was sent. Something \
                 was misread. Start the scan again."
            ),
        }
    }
}

impl std::error::Error for AirError {}

// ------------------------------------------------------------------ intent ---

/// What the online side is asking the cold wallet to sign.
///
/// Carries nothing secret: an address someone gave you, an amount, a fee, the
/// chain height the fee was chosen for, and a nonce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub network: NetworkId,
    pub to: Address,
    pub amount_darks: u64,
    pub fee_darks: u64,
    /// Chain height when this was written. The signer shows it so its owner
    /// can see how old the request is, and the fee is judged against it.
    pub tip_height: u64,
    /// 32 bytes, hex. Ties the answer to this request and nothing else.
    pub nonce: String,
    pub expires_unix: u64,
}

impl Intent {
    /// Refuse a request this wallet must not sign.
    ///
    /// Network first, because it is the one mistake an address cannot warn
    /// about, and the one that spends real money by accident.
    pub fn check(&self, wallet: NetworkId, now_unix: u64) -> Result<(), AirError> {
        if self.network != wallet {
            return Err(AirError::WrongNetwork {
                package: self.network,
                wallet,
            });
        }
        if now_unix > self.expires_unix {
            return Err(AirError::Expired {
                expired_at: self.expires_unix,
                now: now_unix,
            });
        }
        Ok(())
    }

    pub fn to_text(&self) -> String {
        format!(
            "{SCHEME}:intent:{VERSION}|network={}|to={}|amount={}|fee={}|tip={}|nonce={}|expires={}",
            self.network.as_str(),
            self.to.encode(),
            self.amount_darks,
            self.fee_darks,
            self.tip_height,
            self.nonce,
            self.expires_unix,
        )
    }

    pub fn parse(text: &str) -> Result<Self, AirError> {
        let fields = split_package(text, "intent")?;
        let intent = Self {
            network: network_from(take(&fields, "network")?)?,
            to: Address::decode(take(&fields, "to")?).map_err(|_| AirError::BadAddress)?,
            amount_darks: number(take(&fields, "amount")?, "amount")?,
            fee_darks: number(take(&fields, "fee")?, "fee")?,
            tip_height: number(take(&fields, "tip")?, "tip")?,
            nonce: nonce_from(take(&fields, "nonce")?)?,
            expires_unix: number(take(&fields, "expires")?, "expires")?,
        };
        if intent.amount_darks > MAX_SUPPLY_DARKS || intent.fee_darks > MAX_SUPPLY_DARKS {
            return Err(AirError::AmountTooLarge);
        }
        refuse_unknown(&fields, &["network", "to", "amount", "fee", "tip", "nonce", "expires"])?;
        Ok(intent)
    }
}

// ------------------------------------------------------------------ signed ---

/// The finished transaction coming back from the cold wallet.
///
/// The transaction itself is opaque. This module's job is to carry it and to
/// prove it arrived whole; understanding it belongs to the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    pub network: NetworkId,
    /// The nonce of the intent this answers.
    pub nonce: String,
    pub payload: Vec<u8>,
}

impl Signed {
    pub fn to_text(&self) -> String {
        format!(
            "{SCHEME}:signed:{VERSION}|network={}|nonce={}|tx={}",
            self.network.as_str(),
            self.nonce,
            hex::encode(&self.payload),
        )
    }

    pub fn parse(text: &str) -> Result<Self, AirError> {
        let fields = split_package(text, "signed")?;
        let payload = hex::decode(take(&fields, "tx")?).map_err(|_| AirError::BadFrame)?;
        if payload.is_empty() {
            return Err(AirError::PayloadEmpty);
        }
        if payload.len() > MAX_PAYLOAD {
            return Err(AirError::PayloadTooLarge(payload.len()));
        }
        refuse_unknown(&fields, &["network", "nonce", "tx"])?;
        Ok(Self {
            network: network_from(take(&fields, "network")?)?,
            nonce: nonce_from(take(&fields, "nonce")?)?,
            payload,
        })
    }

    /// Accept this only as the answer to `intent`, on this wallet's network,
    /// and only once.
    ///
    /// All three checks in one place because leaving any of them to the caller
    /// is leaving it to be forgotten.
    pub fn accept(
        &self,
        intent: &Intent,
        wallet: NetworkId,
        log: &mut NonceLog,
    ) -> Result<(), AirError> {
        if self.network != wallet {
            return Err(AirError::WrongNetwork {
                package: self.network,
                wallet,
            });
        }
        if self.nonce != intent.nonce {
            return Err(AirError::NonceMismatch);
        }
        if !log.remember(&self.nonce) {
            return Err(AirError::ReplayedNonce);
        }
        Ok(())
    }
}

/// Nonces already used, so the same package cannot be acted on twice.
///
/// Bounded and in order: a till or a wallet left running for a year must not
/// grow a list without end, and the oldest entries are the ones least likely
/// to be re-presented.
#[derive(Clone, Debug, Default)]
pub struct NonceLog {
    seen: Vec<String>,
}

impl NonceLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a nonce. False when it was already there — the caller must then
    /// refuse whatever it was about to do.
    pub fn remember(&mut self, nonce: &str) -> bool {
        if self.seen.iter().any(|n| n == nonce) {
            return false;
        }
        if self.seen.len() >= NONCE_MEMORY {
            self.seen.remove(0);
        }
        self.seen.push(nonce.to_owned());
        true
    }

    pub fn contains(&self, nonce: &str) -> bool {
        self.seen.iter().any(|n| n == nonce)
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

// --------------------------------------------------------------- transport ---

/// The digest of a payload, as the frames carry it.
fn digest_of(payload: &[u8]) -> String {
    hex::encode(&hash_multi(b"nightfall:air:v1", &[payload]).0[..6])
}

/// Cut a payload into frames small enough to photograph.
///
/// Every frame carries the digest of the *whole* payload, not of itself. A
/// per-frame checksum tells you a frame was read correctly; it does not tell
/// you the frames belong together, and splicing two transfers is the failure
/// worth preventing.
pub fn frames(payload: &[u8]) -> Result<Vec<String>, AirError> {
    if payload.is_empty() {
        return Err(AirError::PayloadEmpty);
    }
    if payload.len() > MAX_PAYLOAD {
        return Err(AirError::PayloadTooLarge(payload.len()));
    }
    let digest = digest_of(payload);
    let chunks: Vec<&[u8]> = payload.chunks(MAX_FRAME_PAYLOAD).collect();
    if chunks.len() > MAX_FRAMES {
        return Err(AirError::TooManyFrames(chunks.len()));
    }
    let total = chunks.len();
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            format!(
                "{SCHEME}:frame:{VERSION}|{digest}|{}|{total}|{}",
                index + 1,
                hex::encode(chunk),
            )
        })
        .collect())
}

/// Collects frames until a transfer is whole.
#[derive(Clone, Debug, Default)]
pub struct Reassembler {
    digest: Option<String>,
    total: usize,
    parts: BTreeMap<usize, Vec<u8>>,
}

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one frame. Repeats are ignored; frames from another transfer are
    /// refused rather than mixed in.
    pub fn accept(&mut self, frame: &str) -> Result<(), AirError> {
        if frame.len() > MAX_FRAME_LEN {
            return Err(AirError::BadFrame);
        }
        let rest = frame
            .trim()
            .strip_prefix(&format!("{SCHEME}:frame:"))
            .ok_or(AirError::NotAnAirPackage)?;
        let mut parts = rest.split('|');
        let version: u32 = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or(AirError::BadFrame)?;
        if version != VERSION {
            return Err(AirError::UnknownVersion(version));
        }
        let digest = parts.next().ok_or(AirError::BadFrame)?.to_owned();
        let seq: usize = parse_index(parts.next().ok_or(AirError::BadFrame)?)?;
        let total: usize = parse_index(parts.next().ok_or(AirError::BadFrame)?)?;
        let body = hex::decode(parts.next().ok_or(AirError::BadFrame)?)
            .map_err(|_| AirError::BadFrame)?;
        if parts.next().is_some() || body.is_empty() || body.len() > MAX_FRAME_PAYLOAD {
            return Err(AirError::BadFrame);
        }
        if total == 0 || total > MAX_FRAMES || seq == 0 || seq > total {
            return Err(AirError::BadFrame);
        }

        match &self.digest {
            None => {
                self.digest = Some(digest);
                self.total = total;
            }
            // A frame whose digest or count disagrees is from a different
            // transfer, however plausible its sequence number looks.
            Some(known) if *known != digest || self.total != total => {
                return Err(AirError::FrameFromAnotherTransfer)
            }
            Some(_) => {}
        }
        self.parts.entry(seq).or_insert(body);
        Ok(())
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn have(&self) -> usize {
        self.parts.len()
    }

    /// Which frames are still wanted, so a person can be told what to point
    /// the camera at rather than simply that it is not finished.
    pub fn missing(&self) -> Vec<usize> {
        (1..=self.total)
            .filter(|seq| !self.parts.contains_key(seq))
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.total > 0 && self.parts.len() == self.total
    }

    /// The payload, once every frame is in and the whole thing checks out.
    pub fn finish(&self) -> Result<Vec<u8>, AirError> {
        if !self.is_complete() {
            return Err(AirError::IncompleteTransfer {
                have: self.parts.len(),
                total: self.total,
            });
        }
        let payload: Vec<u8> = self.parts.values().flatten().copied().collect();
        if Some(digest_of(&payload)) != self.digest {
            return Err(AirError::DigestMismatch);
        }
        Ok(payload)
    }
}

// ----------------------------------------------------------------- parsing ---

fn split_package<'a>(
    text: &'a str,
    kind: &str,
) -> Result<BTreeMap<&'a str, &'a str>, AirError> {
    let text = text.trim();
    let rest = text
        .strip_prefix(&format!("{SCHEME}:"))
        .ok_or(AirError::NotAnAirPackage)?;
    let (found_kind, rest) = rest.split_once(':').ok_or(AirError::NotAnAirPackage)?;
    if found_kind != kind {
        return Err(AirError::UnknownKind(found_kind.to_owned()));
    }
    let (version, rest) = rest.split_once('|').ok_or(AirError::NotAnAirPackage)?;
    let version: u32 = version.parse().map_err(|_| AirError::NotAnAirPackage)?;
    if version != VERSION {
        return Err(AirError::UnknownVersion(version));
    }
    let mut fields = BTreeMap::new();
    for pair in rest.split('|').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').ok_or(AirError::NotAnAirPackage)?;
        // Repeated rather than last-wins: two amounts in one package means
        // nobody knows which one was meant.
        if fields.insert(key, value).is_some() {
            return Err(AirError::RepeatedField(key.to_owned()));
        }
    }
    Ok(fields)
}

fn take<'a>(fields: &BTreeMap<&'a str, &'a str>, key: &'static str) -> Result<&'a str, AirError> {
    fields.get(key).copied().ok_or(AirError::MissingField(key))
}

fn refuse_unknown(fields: &BTreeMap<&str, &str>, known: &[&str]) -> Result<(), AirError> {
    for key in fields.keys() {
        if !known.contains(key) {
            return Err(AirError::UnknownField((*key).to_owned()));
        }
    }
    Ok(())
}

fn network_from(value: &str) -> Result<NetworkId, AirError> {
    match value {
        "mainnet" => Ok(NetworkId::Mainnet),
        "testnet" => Ok(NetworkId::Testnet),
        "devnet" => Ok(NetworkId::Devnet),
        _ => Err(AirError::MissingField("network")),
    }
}

/// Digits only, and one spelling. `007` and `+7` are refused for the same
/// reason they are in a payment request: a number with two spellings is two
/// numbers as far as a person comparing them is concerned.
fn number(value: &str, field: &'static str) -> Result<u64, AirError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AirError::BadNumber(field));
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(AirError::BadNumber(field));
    }
    value.parse().map_err(|_| AirError::BadNumber(field))
}

fn parse_index(value: &str) -> Result<usize, AirError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AirError::BadFrame);
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(AirError::BadFrame);
    }
    value.parse().map_err(|_| AirError::BadFrame)
}

fn nonce_from(value: &str) -> Result<String, AirError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AirError::BadNumber("nonce"));
    }
    Ok(value.to_ascii_lowercase())
}

/// A fresh nonce for a new request.
pub fn new_nonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;

    fn address() -> Address {
        WalletKeys::from_seed([44; 32]).address()
    }

    fn intent() -> Intent {
        Intent {
            network: NetworkId::Mainnet,
            to: address(),
            amount_darks: 150_000_000,
            fee_darks: 100_000,
            tip_height: 1_284,
            nonce: "ab".repeat(32),
            expires_unix: 2_000,
        }
    }

    #[test]
    fn an_intent_survives_the_gap_and_comes_back_the_same() {
        let original = intent();
        let read = Intent::parse(&original.to_text()).unwrap();
        assert_eq!(read, original);
        assert_eq!(read.to.encode(), address().encode());
    }

    /// The mistake an address cannot warn about.
    #[test]
    fn a_package_for_another_network_is_refused_on_both_sides() {
        let mut testnet = intent();
        testnet.network = NetworkId::Testnet;
        let error = testnet.check(NetworkId::Mainnet, 1_000).unwrap_err();
        assert!(matches!(error, AirError::WrongNetwork { .. }));
        assert!(error.to_string().contains("same string on every network"));

        // …and again when the signed answer comes back.
        let signed = Signed {
            network: NetworkId::Testnet,
            nonce: testnet.nonce.clone(),
            payload: vec![1, 2, 3],
        };
        let mut log = NonceLog::new();
        assert!(matches!(
            signed.accept(&testnet, NetworkId::Mainnet, &mut log).unwrap_err(),
            AirError::WrongNetwork { .. }
        ));
        assert!(log.is_empty(), "a refused package must not be recorded as used");
    }

    #[test]
    fn an_expired_request_is_refused_because_its_fee_is_stale() {
        let intent = intent();
        assert_eq!(intent.check(NetworkId::Mainnet, 1_999), Ok(()));
        let error = intent.check(NetworkId::Mainnet, 2_001).unwrap_err();
        assert!(matches!(error, AirError::Expired { .. }));
        assert!(error.to_string().contains("fee"));
    }

    /// A signed transaction answers one request and no other.
    #[test]
    fn a_signed_package_cannot_be_paired_with_a_different_request() {
        let first = intent();
        let mut second = intent();
        second.nonce = "cd".repeat(32);

        let signed = Signed {
            network: NetworkId::Mainnet,
            nonce: first.nonce.clone(),
            payload: vec![9; 40],
        };
        let mut log = NonceLog::new();
        assert_eq!(
            signed.accept(&second, NetworkId::Mainnet, &mut log),
            Err(AirError::NonceMismatch),
        );
        assert!(log.is_empty());
        assert_eq!(signed.accept(&first, NetworkId::Mainnet, &mut log), Ok(()));
    }

    /// The QR left on a screen, photographed twice.
    #[test]
    fn the_same_signed_package_is_only_acted_on_once() {
        let intent = intent();
        let signed = Signed {
            network: NetworkId::Mainnet,
            nonce: intent.nonce.clone(),
            payload: vec![7; 64],
        };
        let mut log = NonceLog::new();
        assert_eq!(signed.accept(&intent, NetworkId::Mainnet, &mut log), Ok(()));
        let again = signed.accept(&intent, NetworkId::Mainnet, &mut log).unwrap_err();
        assert_eq!(again, AirError::ReplayedNonce);
        assert!(again.to_string().contains("Nothing was done"));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn the_nonce_log_is_bounded_and_forgets_the_oldest_first() {
        let mut log = NonceLog::new();
        for i in 0..NONCE_MEMORY + 10 {
            assert!(log.remember(&format!("{i:064x}")));
        }
        assert_eq!(log.len(), NONCE_MEMORY);
        assert!(!log.contains(&format!("{:064x}", 0)), "the oldest fell off");
        assert!(log.contains(&format!("{:064x}", NONCE_MEMORY + 9)));
    }

    #[test]
    fn a_signed_package_survives_the_gap() {
        let signed = Signed {
            network: NetworkId::Devnet,
            nonce: "12".repeat(32),
            payload: (0..=255u8).collect(),
        };
        assert_eq!(Signed::parse(&signed.to_text()).unwrap(), signed);
    }

    // -------------------------------------------------------------- frames ---

    #[test]
    fn frames_go_back_together_in_any_order() {
        let payload: Vec<u8> = (0..2_500u32).map(|i| (i % 251) as u8).collect();
        let mut cut = frames(&payload).unwrap();
        assert_eq!(cut.len(), 5);
        cut.reverse();

        let mut reader = Reassembler::new();
        for frame in &cut {
            reader.accept(frame).unwrap();
        }
        assert!(reader.is_complete());
        assert_eq!(reader.finish().unwrap(), payload);
    }

    #[test]
    fn a_repeated_frame_changes_nothing_and_a_missing_one_is_named() {
        let payload: Vec<u8> = (0..1_800u32).map(|i| i as u8).collect();
        let cut = frames(&payload).unwrap();
        assert_eq!(cut.len(), 4);

        let mut reader = Reassembler::new();
        reader.accept(&cut[0]).unwrap();
        reader.accept(&cut[0]).unwrap();
        reader.accept(&cut[0]).unwrap();
        assert_eq!(reader.have(), 1);
        reader.accept(&cut[3]).unwrap();
        assert_eq!(reader.missing(), vec![2, 3]);
        assert!(!reader.is_complete());
        assert_eq!(
            reader.finish(),
            Err(AirError::IncompleteTransfer { have: 2, total: 4 }),
        );

        reader.accept(&cut[1]).unwrap();
        reader.accept(&cut[2]).unwrap();
        assert_eq!(reader.finish().unwrap(), payload);
    }

    /// Two transfers in front of one camera.
    #[test]
    fn frames_from_another_transfer_are_refused_rather_than_spliced() {
        let a: Vec<u8> = vec![1; 1_200];
        let b: Vec<u8> = vec![2; 1_200];
        let cut_a = frames(&a).unwrap();
        let cut_b = frames(&b).unwrap();
        assert_eq!(cut_a.len(), cut_b.len(), "same shape, different contents");

        let mut reader = Reassembler::new();
        reader.accept(&cut_a[0]).unwrap();
        assert_eq!(
            reader.accept(&cut_b[1]),
            Err(AirError::FrameFromAnotherTransfer),
        );
        reader.accept(&cut_a[1]).unwrap();
        reader.accept(&cut_a[2]).unwrap();
        assert_eq!(reader.finish().unwrap(), a);
    }

    /// A frame read wrongly by the camera must not become a short payload.
    #[test]
    fn a_misread_frame_is_caught_by_the_digest_and_not_handed_back() {
        let payload: Vec<u8> = (0..900u32).map(|i| i as u8).collect();
        let cut = frames(&payload).unwrap();
        // Same digest and count, one byte of body different — the shape a
        // misread produces, and the one a per-frame checksum would miss.
        let broken = cut[1].replace("|0001", "|0002");
        let tampered = if broken == cut[1] {
            let (head, body) = cut[1].rsplit_once('|').unwrap();
            let mut bytes = hex::decode(body).unwrap();
            bytes[0] ^= 0xff;
            format!("{head}|{}", hex::encode(bytes))
        } else {
            broken
        };

        let mut reader = Reassembler::new();
        reader.accept(&cut[0]).unwrap();
        reader.accept(&tampered).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.finish(), Err(AirError::DigestMismatch));
    }

    #[test]
    fn malformed_frames_are_refused_before_anything_is_believed() {
        let mut reader = Reassembler::new();
        for bad in [
            "",
            "hello",
            "nightfall-air:frame:1",
            "nightfall-air:frame:1|deadbeef|1|1",
            "nightfall-air:frame:1|deadbeef|0|1|aa",
            "nightfall-air:frame:1|deadbeef|2|1|aa",
            "nightfall-air:frame:1|deadbeef|1|0|aa",
            "nightfall-air:frame:1|deadbeef|1|1|zz",
            "nightfall-air:frame:1|deadbeef|1|1|",
            "nightfall-air:frame:1|deadbeef|01|1|aa",
            "nightfall-air:frame:9|deadbeef|1|1|aa",
            &format!("nightfall-air:frame:1|deadbeef|1|{}|aa", MAX_FRAMES + 1),
            &"x".repeat(MAX_FRAME_LEN + 1),
        ] {
            assert!(reader.accept(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(reader.have(), 0);
    }

    #[test]
    fn a_transfer_is_bounded_at_both_ends() {
        assert_eq!(frames(&[]), Err(AirError::PayloadEmpty));
        let huge = vec![0u8; MAX_PAYLOAD + 1];
        assert_eq!(frames(&huge), Err(AirError::PayloadTooLarge(MAX_PAYLOAD + 1)));
        // The largest payload Air does carry still fits the frame budget.
        let full = vec![0u8; MAX_PAYLOAD];
        assert_eq!(frames(&full).unwrap().len(), MAX_FRAMES);
    }

    // ------------------------------------------------------------- parsing ---

    #[test]
    fn malformed_packages_are_refused_before_anything_is_understood() {
        let good = intent().to_text();
        let cases: Vec<(String, &str)> = vec![
            ("".into(), "empty"),
            ("nightfall:nf1abc".into(), "a payment request, not this"),
            (good.replace("nightfall-air:intent", "nightfall-air:signed"), "wrong kind"),
            (good.replace(":intent:1|", ":intent:9|"), "unknown version"),
            (good.replace("|fee=100000", ""), "missing fee"),
            (format!("{good}|priority=high"), "unknown field"),
            (format!("{good}|fee=1"), "repeated field"),
            (good.replace("amount=150000000", "amount=0150000000"), "two spellings"),
            (good.replace("amount=150000000", "amount=+1"), "signed"),
            (good.replace("amount=150000000", "amount="), "empty amount"),
            (good.replace("network=mainnet", "network=regtest"), "unknown network"),
            (good.replace("to=nf1", "to=zz1"), "bad address"),
            (good.replace(&"ab".repeat(32), "abc"), "short nonce"),
            (
                good.replace("amount=150000000", &format!("amount={}", MAX_SUPPLY_DARKS + 1)),
                "more than will exist",
            ),
        ];
        for (text, why) in cases {
            assert!(Intent::parse(&text).is_err(), "accepted {why}: {text}");
        }
        // …and the good one still parses, so the sweep is a filter and not a
        // closed door.
        assert!(Intent::parse(&good).is_ok());
    }

    /// The whole crossing, with a real wallet and a real transaction.
    ///
    /// Online writes an intent; the cold side parses it, checks it for itself,
    /// and builds a payment from the *parsed* values — never from a summary
    /// somebody else wrote; the result is cut into frames, shuffled as a
    /// camera would deliver them, put back together, and accepted by the
    /// online side exactly once.
    #[test]
    fn a_payment_crosses_the_gap_and_comes_back_once() {
        use nightfall_consensus::{Block, BlockHeader};
        use nightfall_ledger::{build_coinbase, BlockBody, LedgerState};
        use nightfall_types::{Hash256, Height, DARKS_PER_NIGHT, PROTOCOL_VERSION};

        const NETWORK: NetworkId = NetworkId::Devnet;
        let ctx = NETWORK.proof_context();
        let reward = 20 * DARKS_PER_NIGHT;

        // The cold wallet: holds the seed, has scanned, never sees a network.
        let mut cold = crate::Wallet::in_memory(NETWORK, WalletKeys::from_seed([55; 32]), 0);
        let coinbase = build_coinbase(&cold.address(), reward, 0, ctx).unwrap();
        let body = BlockBody::aggregate(&[coinbase]);
        let mut ledger = LedgerState::genesis();
        ledger.apply_block(&body, Height(0), reward, ctx).unwrap();
        cold.scan_blocks(&[Block {
            header: BlockHeader {
                version: PROTOCOL_VERSION,
                height: Height(0),
                prev_hash: Hash256::ZERO,
                utxo_root: ledger.utxo_root(),
                kernel_sum: ledger.kernel_sum(),
                body_root: body.hash(),
                timestamp_unix: 1_800_000_000,
                difficulty: 1,
                nonce: 0,
                reward_darks: reward,
            },
            body,
        }])
        .unwrap();

        // Online side writes the request. No secret in it.
        let payee = WalletKeys::from_seed([56; 32]).address();
        let asked = Intent {
            network: NETWORK,
            to: payee,
            amount_darks: 3 * DARKS_PER_NIGHT,
            fee_darks: DARKS_PER_NIGHT / 1_000,
            tip_height: 2_000,
            nonce: new_nonce(),
            expires_unix: 9_000,
        };
        let over_the_gap = asked.to_text();
        assert!(
            !over_the_gap.contains(&hex::encode(cold.keys.seed)),
            "an intent must carry nothing secret",
        );

        // Cold side: parse, check for itself, then build from what it parsed.
        let read = Intent::parse(&over_the_gap).unwrap();
        read.check(NETWORK, 1_000).unwrap();
        assert_eq!(read.to.encode(), payee.encode());
        let tx = cold
            .create_payment_at(
                &read.to,
                read.amount_darks,
                read.fee_darks,
                "",
                read.tip_height,
                1_440,
            )
            .unwrap();
        let signed = Signed {
            network: NETWORK,
            nonce: read.nonce.clone(),
            payload: serde_json::to_vec(&tx).unwrap(),
        };

        // …across the gap as frames, delivered out of order and with repeats.
        let text = signed.to_text();
        let cut = frames(text.as_bytes()).unwrap();
        assert!(cut.len() > 1, "a real transaction needs more than one frame");
        let mut reader = Reassembler::new();
        for frame in cut.iter().rev().chain(cut.iter()) {
            reader.accept(frame).unwrap();
        }
        let rebuilt = String::from_utf8(reader.finish().unwrap()).unwrap();
        assert_eq!(rebuilt, text);

        // Online side accepts it once, and refuses the second photograph.
        let back = Signed::parse(&rebuilt).unwrap();
        let mut log = NonceLog::new();
        assert_eq!(back.accept(&asked, NETWORK, &mut log), Ok(()));
        assert_eq!(
            back.accept(&asked, NETWORK, &mut log),
            Err(AirError::ReplayedNonce),
        );

        // …and what arrived is the transaction the cold wallet built.
        let delivered: nightfall_ledger::Transaction =
            serde_json::from_slice(&back.payload).unwrap();
        assert_eq!(delivered.txid(), tx.txid());
    }

    #[test]
    fn a_fresh_nonce_is_thirty_two_bytes_and_not_the_same_twice() {
        let a = new_nonce();
        let b = new_nonce();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(nonce_from(&a).unwrap(), a);
    }
}
