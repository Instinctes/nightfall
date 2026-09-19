//! Counter: a till that watches for payments it asked for.
//!
//! A merchant writes a payment request, hands it over, and then has one
//! question the wallet has never been able to answer: *did that one get paid?*
//! A balance cannot answer it. A list of incoming payments cannot answer it
//! either, because two customers who each owe five NIGHT produce two identical
//! rows.
//!
//! # The rule this module exists to enforce
//!
//! **An invoice is never settled by an amount alone.** That is the whole
//! design. If matching went by amount, then a till holding two open invoices
//! for the same figure would close whichever one it looked at first, and the
//! other customer would be asked to pay twice for a payment that had already
//! arrived. Worse, it would happen silently and look correct.
//!
//! So a payment settles an invoice only when it carries that invoice's
//! reference in its memo, exactly. The reference travels because the payment
//! request carries it: a payer's wallet fills the memo from the request, and
//! that memo arrives with the output. Everything else — a payment for the
//! right amount, in the right window, with no reference — is offered to the
//! merchant as a *candidate* and never applied. A till that guesses is worse
//! than a till that asks.
//!
//! # What a till is allowed to know
//!
//! Everything here is computed from what a **view key** can see: incoming
//! outputs, their amounts, their memos, their heights. Nothing in this module
//! needs the spend key, and nothing in it may be given one — a till sits on a
//! counter in a shop, which is the least defensible computer a business owns.
//!
//! That is a property of this code, not yet of a deployment: Core still holds
//! a spend key, and a separate view-only till process is not built. The
//! property is worth keeping true from the start, because it is very hard to
//! add later.
//!
//! A view key is also not an access token. It cannot be revoked, and it
//! reveals every amount and memo the wallet ever received — so it must never
//! be handed to a till over a network or exposed by one, and this module never
//! serialises it.
//!
//! # Expiry is the merchant's policy and nothing more
//!
//! A payment that arrives after the deadline is a payment. The chain has no
//! opinion about an invoice, the coins are really there, and a till that
//! reported "expired" while holding the customer's money would be lying about
//! the only thing that matters. So a late payment settles the invoice and is
//! flagged as late; only an invoice with *no* payment at all goes to
//! [`InvoiceState::Expired`], and even then a payment afterwards settles it.

use nightfall_types::{Amount, DARKS_PER_NIGHT, MAX_SUPPLY_DARKS};
use serde::{Deserialize, Serialize};

/// Longest reference a till will accept. Matches the payment request's bound,
/// because the reference has to fit in one.
pub const MAX_REFERENCE: usize = 64;
/// Longest description. Same reason.
pub const MAX_DESCRIPTION: usize = 64;
/// Most invoices one till keeps. A shop that has written ten thousand open
/// invoices has a different problem, and an unbounded list inside the
/// encrypted snapshot would make every save slower for ever.
pub const MAX_INVOICES: usize = 2_000;

/// One thing the merchant asked to be paid for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    /// The merchant's own reference, and the thing a payment must quote to
    /// settle this. Unique within one till.
    pub reference: String,
    /// What was asked for. `None` means the payer chose the amount, which a
    /// till allows: a donation box is a till too.
    pub amount_darks: Option<u64>,
    pub description: String,
    pub created_unix: u64,
    /// The merchant's own deadline. Never a rule — see the module docs.
    pub expires_unix: Option<u64>,
    /// Set when the merchant closes an invoice by hand, with their reason.
    /// A till must be able to say "settled in cash" without inventing a
    /// payment that never happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_note: Option<String>,
}

/// A payment as a view key sees it. No spend key, no keys at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incoming {
    pub amount_darks: u64,
    pub memo: String,
    /// `None` while still in the mempool.
    pub height: Option<u64>,
    pub timestamp: u64,
    /// The output commitment, for showing the merchant which payment this was.
    pub reference_id: String,
}

/// Where an invoice stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvoiceState {
    /// Asked for, nothing quoted its reference, deadline not passed.
    Open,
    /// Nothing quoted its reference and the merchant's deadline has passed.
    /// A payment afterwards still settles it.
    Expired,
    /// Paid in full, or more than asked with the difference named.
    Paid { late: bool },
    /// Less arrived than was asked for. The shortfall is what the merchant
    /// needs to say out loud, so it is carried rather than recomputed.
    Underpaid { short_darks: u64, late: bool },
    Overpaid { extra_darks: u64, late: bool },
    /// The merchant closed it themselves.
    Closed,
}

/// An invoice and what has happened to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvoiceStatus {
    pub state: InvoiceState,
    /// Payments that quoted this invoice's reference. Summed into the state.
    pub matched: Vec<Incoming>,
    /// Payments that did *not* quote it but could plausibly be meant for it:
    /// right amount, inside the window, reference not recognised. Shown to the
    /// merchant, never applied. See the module docs for why.
    pub candidates: Vec<Incoming>,
    pub received_darks: u64,
}

impl InvoiceStatus {
    /// A short line for a till display, written here so that every screen
    /// showing a till says the same thing about the same invoice.
    pub fn headline(&self) -> String {
        match &self.state {
            InvoiceState::Open => "Waiting".to_owned(),
            InvoiceState::Expired => "Past your deadline, unpaid".to_owned(),
            InvoiceState::Closed => "Closed by you".to_owned(),
            InvoiceState::Paid { late } => {
                if *late {
                    "Paid, after your deadline".to_owned()
                } else {
                    "Paid".to_owned()
                }
            }
            InvoiceState::Underpaid { short_darks, .. } => {
                format!("Short by {}", Amount(*short_darks))
            }
            InvoiceState::Overpaid { extra_darks, .. } => {
                format!("Overpaid by {}", Amount(*extra_darks))
            }
        }
    }

    /// True when the merchant should hand over the goods.
    ///
    /// Deliberately narrow: an underpayment is not a payment, and a candidate
    /// is not a payment at all. Anything else is the merchant's judgement and
    /// this function must not make it for them.
    pub fn is_settled(&self) -> bool {
        matches!(
            self.state,
            InvoiceState::Paid { .. } | InvoiceState::Overpaid { .. } | InvoiceState::Closed
        )
    }
}

impl Invoice {
    /// Refuse an invoice a till cannot hold honestly.
    pub fn check(&self) -> Result<(), String> {
        let reference = self.reference.trim();
        if reference.is_empty() {
            return Err(
                "Give this invoice a reference. It is what a payment quotes to \
                 settle it, and without one nothing can be matched to it."
                    .to_owned(),
            );
        }
        if reference.chars().count() > MAX_REFERENCE {
            return Err(format!(
                "A reference is at most {MAX_REFERENCE} characters, so it fits in \
                 a payment request."
            ));
        }
        if self.description.chars().count() > MAX_DESCRIPTION {
            return Err(format!(
                "A description is at most {MAX_DESCRIPTION} characters."
            ));
        }
        match self.amount_darks {
            Some(0) => {
                return Err(
                    "An invoice for zero is not an invoice. Leave the amount \
                     empty if the payer chooses it."
                        .to_owned(),
                )
            }
            Some(darks) if darks > MAX_SUPPLY_DARKS => {
                return Err("That is more NIGHT than will ever exist.".to_owned())
            }
            _ => {}
        }
        Ok(())
    }
}

/// Work out where one invoice stands, from the payments a view key can see.
///
/// `now_unix` is the reader's clock, passed in rather than read here, because
/// a deadline is measured against whoever is looking.
pub fn status(invoice: &Invoice, payments: &[Incoming], now_unix: u64) -> InvoiceStatus {
    let reference = invoice.reference.trim();
    let mut matched = Vec::new();
    let mut candidates = Vec::new();

    for payment in payments {
        // Exact, after trimming. Not a prefix and not case-folded: a reference
        // is an identifier the merchant chose, and two references that differ
        // only in case are two invoices, not one.
        if payment.memo.trim() == reference {
            matched.push(payment.clone());
            continue;
        }
        // A candidate is an unclaimed payment for exactly the right figure. It
        // is shown so the merchant can recognise it; it never moves the state.
        if let Some(asked) = invoice.amount_darks {
            let unclaimed = !looks_like_another_reference(&payment.memo);
            if unclaimed && payment.amount_darks == asked && payment.timestamp >= invoice.created_unix
            {
                candidates.push(payment.clone());
            }
        }
    }

    let received_darks: u64 = matched.iter().map(|p| p.amount_darks).fold(0, u64::saturating_add);
    // Late is a property of the payments, not of the clock at read time: an
    // invoice paid on time does not become "late" because it is looked at a
    // week afterwards.
    let late = invoice.expires_unix.is_some_and(|deadline| {
        matched.iter().any(|p| p.timestamp > deadline)
    });

    let state = if invoice.closed_note.is_some() {
        InvoiceState::Closed
    } else if matched.is_empty() {
        if invoice.expires_unix.is_some_and(|deadline| now_unix > deadline) {
            InvoiceState::Expired
        } else {
            InvoiceState::Open
        }
    } else {
        match invoice.amount_darks {
            // A donation box: anything that arrived quoting the reference is
            // the whole of what was asked for.
            None => InvoiceState::Paid { late },
            Some(asked) if received_darks == asked => InvoiceState::Paid { late },
            Some(asked) if received_darks < asked => InvoiceState::Underpaid {
                short_darks: asked - received_darks,
                late,
            },
            Some(asked) => InvoiceState::Overpaid {
                extra_darks: received_darks - asked,
                late,
            },
        }
    };

    // A candidate is a question: "is this unidentified payment meant for this
    // invoice?" Once the invoice is settled there is no question left, and
    // asking it anyway tells a merchant something is unaccounted for when
    // nothing is. Seen on a real till: an invoice marked Paid still carried
    // "1 payment for this amount arrived without quoting the reference".
    if matches!(
        state,
        InvoiceState::Paid { .. } | InvoiceState::Overpaid { .. } | InvoiceState::Closed
    ) {
        candidates.clear();
    }

    InvoiceStatus {
        state,
        matched,
        candidates,
        received_darks,
    }
}

/// A memo that is plainly somebody else's reference should not be offered as a
/// candidate for this invoice.
///
/// Deliberately crude, and deliberately conservative in the safe direction: it
/// only suppresses a suggestion. Being wrong here costs the merchant one row
/// to ignore; the expensive mistake — applying it — is not available to this
/// function at all.
fn looks_like_another_reference(memo: &str) -> bool {
    let memo = memo.trim();
    !memo.is_empty() && !memo.contains(' ') && memo.chars().count() <= MAX_REFERENCE
}

/// The total a till has taken in, from settled invoices only.
///
/// Underpayments are excluded on purpose. A day's takings that counted a half
/// payment as a sale is a day's takings that will not reconcile, and the
/// merchant finds out at the end of the day rather than at the counter.
pub fn takings_darks(statuses: &[InvoiceStatus]) -> u64 {
    statuses
        .iter()
        .filter(|s| matches!(s.state, InvoiceState::Paid { .. } | InvoiceState::Overpaid { .. }))
        .map(|s| s.received_darks)
        .fold(0, u64::saturating_add)
}

/// A payment request for an invoice, so the two cannot drift apart.
///
/// The request's `id` *is* the invoice reference: that is the channel by which
/// a payment comes back carrying something this till can match. Building the
/// request anywhere else would let the two disagree.
pub fn request_for(
    invoice: &Invoice,
    address: nightfall_crypto::Address,
    network: nightfall_types::NetworkId,
) -> crate::payment_request::PaymentRequest {
    crate::payment_request::PaymentRequest {
        address,
        network,
        amount_darks: invoice.amount_darks,
        // The memo is what reaches the payee, so the reference goes there. The
        // description is for the customer to read, and is appended only when
        // there is room for both without losing the reference.
        memo: invoice.reference.trim().to_owned(),
        invoice: invoice.reference.trim().to_owned(),
        expires_unix: invoice.expires_unix,
    }
}

/// Whole NIGHT, for a till display that has no room for eight decimals.
pub fn round_night(darks: u64) -> String {
    format!("{}.{:02}", darks / DARKS_PER_NIGHT, (darks % DARKS_PER_NIGHT) / 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoice(reference: &str, amount: Option<u64>) -> Invoice {
        Invoice {
            reference: reference.into(),
            amount_darks: amount,
            description: "two coffees".into(),
            created_unix: 1_000,
            expires_unix: Some(2_000),
            closed_note: None,
        }
    }

    fn paid(amount: u64, memo: &str, at: u64) -> Incoming {
        Incoming {
            amount_darks: amount,
            memo: memo.into(),
            height: Some(7),
            timestamp: at,
            reference_id: format!("{memo}-{at}"),
        }
    }

    #[test]
    fn a_payment_quoting_the_reference_settles_the_invoice() {
        let inv = invoice("A-17", Some(500));
        let s = status(&inv, &[paid(500, "A-17", 1_500)], 1_600);
        assert_eq!(s.state, InvoiceState::Paid { late: false });
        assert!(s.is_settled());
        assert_eq!(s.received_darks, 500);
        assert_eq!(s.headline(), "Paid");
    }

    /// The reason this module exists.
    ///
    /// Two customers owe the same figure. A till that matched on amount would
    /// close whichever invoice it looked at first and then ask the other
    /// customer to pay again for money that had already arrived — silently,
    /// and looking correct the whole time.
    #[test]
    fn two_invoices_for_the_same_amount_do_not_settle_each_other() {
        let a = invoice("A-17", Some(500));
        let b = invoice("B-18", Some(500));
        let payments = [paid(500, "A-17", 1_500)];

        let sa = status(&a, &payments, 1_600);
        let sb = status(&b, &payments, 1_600);
        assert_eq!(sa.state, InvoiceState::Paid { late: false });
        assert_eq!(sb.state, InvoiceState::Open, "B must not be settled by A's payment");
        assert_eq!(sb.received_darks, 0);
        // …and B is not even offered it as a candidate, because that memo is
        // recognisably somebody else's reference.
        assert!(sb.candidates.is_empty(), "{:?}", sb.candidates);
    }

    /// A payment with no reference at all is shown and never applied.
    #[test]
    fn an_unreferenced_payment_is_a_candidate_and_moves_nothing() {
        let inv = invoice("A-17", Some(500));
        let s = status(&inv, &[paid(500, "  ", 1_500)], 1_600);
        assert_eq!(s.state, InvoiceState::Open);
        assert_eq!(s.received_darks, 0);
        assert_eq!(s.candidates.len(), 1);
        assert!(!s.is_settled(), "a candidate must never settle anything");

        // A sentence rather than an identifier is also offered — a customer
        // typing "for the coffees" is the ordinary case.
        let s = status(&inv, &[paid(500, "for the coffees", 1_500)], 1_600);
        assert_eq!(s.candidates.len(), 1);
        assert_eq!(s.state, InvoiceState::Open);
    }

    #[test]
    fn part_payments_add_up_and_the_shortfall_is_named() {
        let inv = invoice("A-17", Some(1_000));
        let s = status(&inv, &[paid(400, "A-17", 1_100), paid(300, "A-17", 1_200)], 1_500);
        assert_eq!(
            s.state,
            InvoiceState::Underpaid { short_darks: 300, late: false }
        );
        assert_eq!(s.received_darks, 700);
        assert!(!s.is_settled(), "an underpayment is not a payment");
        assert!(s.headline().starts_with("Short by"));

        let s = status(
            &inv,
            &[paid(400, "A-17", 1_100), paid(300, "A-17", 1_200), paid(300, "A-17", 1_300)],
            1_500,
        );
        assert_eq!(s.state, InvoiceState::Paid { late: false });
    }

    #[test]
    fn overpayment_says_by_how_much_rather_than_just_paid() {
        let inv = invoice("A-17", Some(500));
        let s = status(&inv, &[paid(750, "A-17", 1_500)], 1_600);
        assert_eq!(
            s.state,
            InvoiceState::Overpaid { extra_darks: 250, late: false }
        );
        assert!(s.is_settled(), "the goods are paid for; the change is a separate matter");
        assert!(s.headline().contains("Overpaid by"));
    }

    /// A deadline is the merchant's policy. The coins are really there.
    #[test]
    fn a_late_payment_still_settles_and_is_flagged_rather_than_refused() {
        let inv = invoice("A-17", Some(500));
        let s = status(&inv, &[paid(500, "A-17", 9_999)], 10_000);
        assert_eq!(s.state, InvoiceState::Paid { late: true });
        assert!(s.is_settled());
        assert_eq!(s.headline(), "Paid, after your deadline");

        // …and an invoice nobody paid does expire, but is not closed: a
        // payment tomorrow still settles it.
        let s = status(&inv, &[], 10_000);
        assert_eq!(s.state, InvoiceState::Expired);
        assert!(!s.is_settled());
    }

    /// A settled invoice asks no questions.
    ///
    /// A candidate is the question "is this unidentified payment meant for this
    /// one?". Once the invoice is paid there is nothing left to ask, and
    /// asking anyway tells the merchant something is unaccounted for when
    /// nothing is — which is exactly what a real till showed: an invoice
    /// marked Paid, carrying a warning about a payment it did not need.
    #[test]
    fn a_settled_invoice_stops_offering_candidates() {
        let inv = invoice("A-17", Some(500));
        let stranger = paid(500, "", 1_400);

        let open = status(&inv, &[stranger.clone()], 1_600);
        assert_eq!(open.state, InvoiceState::Open);
        assert_eq!(open.candidates.len(), 1, "an open invoice still asks");

        let settled = status(&inv, &[stranger.clone(), paid(500, "A-17", 1_500)], 1_600);
        assert_eq!(settled.state, InvoiceState::Paid { late: false });
        assert!(settled.candidates.is_empty());

        // …but an invoice that is only part paid still asks, because the
        // unidentified payment might be the rest of it.
        let short = status(&inv, &[stranger.clone(), paid(200, "A-17", 1_500)], 1_600);
        assert!(matches!(short.state, InvoiceState::Underpaid { .. }));
        assert_eq!(short.candidates.len(), 1);

        let mut closed_invoice = inv.clone();
        closed_invoice.closed_note = Some("paid in cash".into());
        assert!(status(&closed_invoice, &[stranger], 1_600).candidates.is_empty());
    }

    #[test]
    fn a_donation_box_takes_whatever_arrives() {
        let inv = invoice("TIPS", None);
        let s = status(&inv, &[paid(3, "TIPS", 1_100)], 1_200);
        assert_eq!(s.state, InvoiceState::Paid { late: false });
        assert_eq!(s.received_darks, 3);
        // With no amount asked for, nothing can be a candidate either.
        let s = status(&inv, &[paid(3, "", 1_100)], 1_200);
        assert!(s.candidates.is_empty());
        assert_eq!(s.state, InvoiceState::Open);
    }

    #[test]
    fn a_merchant_can_close_an_invoice_without_inventing_a_payment() {
        let mut inv = invoice("A-17", Some(500));
        inv.closed_note = Some("paid in cash".into());
        let s = status(&inv, &[], 1_200);
        assert_eq!(s.state, InvoiceState::Closed);
        assert!(s.is_settled());
        assert_eq!(s.received_darks, 0, "closing must not pretend money arrived");
    }

    #[test]
    fn takings_count_settled_invoices_only() {
        let full = status(&invoice("A", Some(500)), &[paid(500, "A", 1_100)], 1_200);
        let over = status(&invoice("B", Some(500)), &[paid(700, "B", 1_100)], 1_200);
        let short = status(&invoice("C", Some(500)), &[paid(200, "C", 1_100)], 1_200);
        let open = status(&invoice("D", Some(500)), &[], 1_200);
        let closed = {
            let mut i = invoice("E", Some(500));
            i.closed_note = Some("cash".into());
            status(&i, &[], 1_200)
        };
        assert_eq!(takings_darks(&[full, over, short, open, closed]), 500 + 700);
    }

    #[test]
    fn an_invoice_a_till_cannot_hold_honestly_is_refused() {
        let ok = invoice("A-17", Some(500));
        assert_eq!(ok.check(), Ok(()));

        for (mutate, expect) in [
            ((|i: &mut Invoice| i.reference = "   ".into()) as fn(&mut Invoice), "reference"),
            (|i| i.reference = "r".repeat(MAX_REFERENCE + 1), "at most"),
            (|i| i.description = "d".repeat(MAX_DESCRIPTION + 1), "at most"),
            (|i| i.amount_darks = Some(0), "zero is not an invoice"),
            (|i| i.amount_darks = Some(MAX_SUPPLY_DARKS + 1), "ever exist"),
        ] {
            let mut bad = ok.clone();
            mutate(&mut bad);
            let error = bad.check().unwrap_err();
            assert!(error.contains(expect), "{error}");
        }
    }

    /// The request a till hands out must carry the reference that comes back,
    /// or nothing can ever be matched. Checked through the real parser.
    #[test]
    fn the_request_carries_the_reference_that_settles_it() {
        use crate::payment_request::PaymentRequest;
        use nightfall_crypto::WalletKeys;
        use nightfall_types::NetworkId;

        let keys = WalletKeys::from_seed([8; 32]);
        let inv = invoice("A-17", Some(150_000_000));
        let request = request_for(&inv, keys.address(), NetworkId::Mainnet);
        let uri = request.to_uri();
        let read = PaymentRequest::parse(&uri).unwrap();
        assert_eq!(read, request);
        assert_eq!(read.invoice, "A-17");
        assert_eq!(read.memo, "A-17", "the memo is the channel back to the till");

        // A payer whose wallet fills the memo from the request settles it.
        let arrival = paid(150_000_000, &read.memo, 1_500);
        assert_eq!(
            status(&inv, &[arrival], 1_600).state,
            InvoiceState::Paid { late: false },
        );
    }
}
