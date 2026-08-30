//! The two warnings that must stay on screen. Tests grep the wallet markup
//! for these exact strings; changing the wording requires changing the test.

pub const NOT_PRIVATE: &str =
    "A swap is not private. It links a NIGHT transaction and a Bitcoin transaction in time and amount.";

pub const NO_NIGHT_REFUND: &str =
    "There is no NIGHT refund. If the other party cancels and never refunds, NIGHT locked in this swap is stuck forever.";

pub const WART: &str = NO_NIGHT_REFUND;
