//! Fee ladder for pre-signed abort transactions.
//!
//! A single baked-in fee cannot be raised later (the sighash binds it).
//! Phase 0 therefore signs several rungs; at broadcast we pick the cheapest
//! that still clears `estimatesmartfee`.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeLadder {
    /// Absolute fees in sats, ascending.
    pub rungs_sats: Vec<u64>,
}

impl FeeLadder {
    pub fn mainnet() -> Self {
        Self {
            rungs_sats: vec![800, 2_000, 5_000, 15_000, 40_000],
        }
    }

    /// First rung ≥ estimate, else the top rung. Never silently drop to 1 sat.
    pub fn pick(&self, estimated_sats: u64) -> u64 {
        self.rungs_sats
            .iter()
            .copied()
            .find(|r| *r >= estimated_sats)
            .unwrap_or(*self.rungs_sats.last().unwrap_or(&estimated_sats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_skips_rungs_that_cannot_clear_the_mempool() {
        let l = FeeLadder::mainnet();
        assert_eq!(l.pick(300), 800, "below the floor still pays the floor");
        assert_eq!(l.pick(2_000), 2_000);
        assert_eq!(l.pick(6_000), 15_000);
        assert_eq!(
            l.pick(1_000_000),
            40_000,
            "above the top we still send the top, not invent a fee"
        );
        // Mutation: returning rungs[0] always would fail the 6_000 case.
        assert_ne!(l.pick(6_000), l.rungs_sats[0]);
    }
}
