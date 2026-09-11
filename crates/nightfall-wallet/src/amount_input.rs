//! One amount, typed by a person, read the same way everywhere.
//!
//! # Why this is a module and not three functions
//!
//! It was three functions. Core, the browser wallet and the mobile wallet each
//! carried their own decimal parser, and they had drifted:
//!
//! | typed | Core          | browser      | mobile       |
//! |-------|---------------|--------------|--------------|
//! | `.5`  | refused       | 0.5 NIGHT    | 0.5 NIGHT    |
//! | `+5`  | refused       | 5 NIGHT      | 5 NIGHT      |
//!
//! `u64::from_str` accepts a leading `+`, which is how the last row happened
//! without anyone writing it down. So the same characters, typed into two
//! wallets from the same project, meant two different amounts of money in one
//! case and an amount versus an error in the other.
//!
//! That is the divergence [`crate::payment_request`] exists to prevent on the
//! wire, arriving instead at the keyboard. This module closes it: one parser,
//! one set of rules, one wording for each refusal. Any wallet that asks a
//! person for an amount calls this.
//!
//! # The rules
//!
//! - Digits, with at most one decimal point, and at most eight places after it
//!   — NIGHT has exactly eight, and a ninth would have to be silently dropped.
//! - A comma is a decimal point. Most of the world writes `1,5`.
//! - No sign. `+5` is refused rather than accepted, because `+5` and `5` would
//!   otherwise be two spellings of one amount, and a payment instruction should
//!   have exactly one.
//! - Zero is refused. A payment of nothing is not a payment, and a request for
//!   nothing is not a request.
//! - Above the whole supply is refused *by name*, rather than reported as "not
//!   a number" — which is what the browser used to say about
//!   `99999999999999999999`, teaching the owner that the wallet was broken
//!   rather than that the amount was impossible.
//!
//! Parsing is done on the characters and never through `f64`: binary floating
//! point cannot hold eight decimal places exactly, and losing a dark in a
//! payment form is not a rounding error, it is somebody's money.

use nightfall_types::{DARKS_PER_NIGHT, MAX_SUPPLY_DARKS};
use std::fmt;

/// The most decimal places NIGHT has.
pub const DECIMALS: usize = 8;

/// Why a typed amount was refused. Each variant is one thing a person did, so
/// that the message can say which thing it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmountError {
    Empty,
    Negative,
    Signed,
    NotDigits,
    TwoPoints,
    TooPrecise,
    Zero,
    TooLarge,
}

impl fmt::Display for AmountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Enter an amount in NIGHT."),
            Self::Negative => write!(f, "An amount cannot be negative."),
            Self::Signed => write!(f, "Write the amount without a leading plus."),
            Self::NotDigits => write!(
                f,
                "An amount is digits, with an optional decimal point — nothing else."
            ),
            Self::TwoPoints => write!(f, "An amount has at most one decimal point."),
            Self::TooPrecise => write!(
                f,
                "NIGHT has {DECIMALS} decimal places and this has more. The extra \
                 would have to be dropped, and dropping part of an amount \
                 quietly is not something this wallet does."
            ),
            Self::Zero => write!(f, "Enter an amount greater than zero."),
            Self::TooLarge => write!(f, "That is more NIGHT than will ever exist."),
        }
    }
}

impl std::error::Error for AmountError {}

/// Read a decimal NIGHT amount as a whole number of darks.
pub fn parse_night(text: &str) -> Result<u64, AmountError> {
    let normalised = text.trim().replace(',', ".");
    if normalised.is_empty() {
        return Err(AmountError::Empty);
    }
    if normalised.starts_with('-') {
        return Err(AmountError::Negative);
    }
    if normalised.starts_with('+') {
        return Err(AmountError::Signed);
    }

    let (whole, frac) = normalised
        .split_once('.')
        .unwrap_or((normalised.as_str(), ""));
    if frac.contains('.') {
        return Err(AmountError::TwoPoints);
    }
    // `.5` and `5.` are both accepted, and both mean what they look like. Only
    // a bare `.` has no digits at all.
    if whole.is_empty() && frac.is_empty() {
        return Err(AmountError::NotDigits);
    }
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(frac) {
        return Err(AmountError::NotDigits);
    }
    if frac.len() > DECIMALS {
        return Err(AmountError::TooPrecise);
    }

    let whole: u64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| AmountError::TooLarge)?
    };
    let mut padded = frac.to_owned();
    while padded.len() < DECIMALS {
        padded.push('0');
    }
    let frac: u64 = padded.parse().expect("eight ascii digits fit in u64");

    let darks = whole
        .checked_mul(DARKS_PER_NIGHT)
        .and_then(|d| d.checked_add(frac))
        .ok_or(AmountError::TooLarge)?;
    if darks == 0 {
        return Err(AmountError::Zero);
    }
    if darks > MAX_SUPPLY_DARKS {
        return Err(AmountError::TooLarge);
    }
    Ok(darks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_amounts_are_exact() {
        for (text, darks) in [
            ("1", DARKS_PER_NIGHT),
            ("0.5", DARKS_PER_NIGHT / 2),
            (".5", DARKS_PER_NIGHT / 2),
            ("5.", 5 * DARKS_PER_NIGHT),
            ("2.25", 2 * DARKS_PER_NIGHT + 25_000_000),
            ("0.00000001", 1),
            ("1,5", DARKS_PER_NIGHT + 50_000_000),
            ("  1.5  ", DARKS_PER_NIGHT + 50_000_000),
            ("0.10000000", DARKS_PER_NIGHT / 10),
            ("00.5", DARKS_PER_NIGHT / 2),
        ] {
            assert_eq!(parse_night(text), Ok(darks), "{text:?}");
        }
    }

    /// The divergence this module was written to end.
    ///
    /// Both of these used to depend on which wallet the person was holding.
    /// Now they are decided once, and the answer is the same in all three.
    #[test]
    fn the_two_spellings_that_used_to_differ_between_wallets() {
        assert_eq!(parse_night(".5"), Ok(DARKS_PER_NIGHT / 2));
        assert_eq!(parse_night("+5"), Err(AmountError::Signed));
    }

    #[test]
    fn every_refusal_says_which_mistake_it_was() {
        for (text, expected) in [
            ("", AmountError::Empty),
            ("   ", AmountError::Empty),
            ("-1", AmountError::Negative),
            ("+5", AmountError::Signed),
            ("abc", AmountError::NotDigits),
            (".", AmountError::NotDigits),
            ("1 000", AmountError::NotDigits),
            ("1_000", AmountError::NotDigits),
            ("0x10", AmountError::NotDigits),
            ("1.2.3", AmountError::TwoPoints),
            ("0.123456789", AmountError::TooPrecise),
            ("0", AmountError::Zero),
            ("0.0", AmountError::Zero),
            ("0.00000000", AmountError::Zero),
            ("99999999999999999999", AmountError::TooLarge),
        ] {
            assert_eq!(parse_night(text), Err(expected), "{text:?}");
        }
    }

    /// A figure larger than every coin that will exist is named as that, not
    /// as a parse failure. `u64` would hold some of these perfectly well, so
    /// the check is against the supply and not against the type.
    #[test]
    fn beyond_the_supply_is_refused_by_name_and_the_supply_itself_is_not() {
        let supply = MAX_SUPPLY_DARKS;
        let whole = supply / DARKS_PER_NIGHT;
        let text = format!("{}.{:08}", whole, supply % DARKS_PER_NIGHT);
        assert_eq!(parse_night(&text), Ok(supply), "the supply itself is payable");
        assert_eq!(
            parse_night(&format!("{}", whole + 1)),
            Err(AmountError::TooLarge),
        );
        assert_eq!(
            format!("{}", AmountError::TooLarge),
            "That is more NIGHT than will ever exist.",
        );
    }

    /// Eight places are kept exactly. This is the reason the parser works on
    /// characters: `0.1 + 0.2` in binary floating point is not `0.3`, and a
    /// payment form is the last place to discover that.
    #[test]
    fn eight_places_survive_that_floating_point_would_not() {
        for places in 1..=DECIMALS {
            let text = format!("0.{}1", "0".repeat(places - 1));
            let expected = 10u64.pow((DECIMALS - places) as u32);
            assert_eq!(parse_night(&text), Ok(expected), "{text}");
        }
        assert_eq!(parse_night("0.30000000"), Ok(30_000_000));
        assert_eq!(
            parse_night("0.1").unwrap() + parse_night("0.2").unwrap(),
            parse_night("0.3").unwrap(),
        );
    }
}
