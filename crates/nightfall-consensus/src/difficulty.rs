//! LWMA-1 difficulty retarget (Zawy12) and the median-time-past rule.
//!
//! Retargeting happens on **every** block over a linearly-weighted window, so
//! the chain absorbs hashrate swings within minutes instead of hours. Recent
//! solve times carry more weight than old ones, which is what makes LWMA
//! resistant to the hash-and-run behaviour small chains attract.

use nightfall_types::{
    DIFFICULTY_WINDOW, LWMA_MAX_SOLVETIME_FACTOR, LWMA_MIN_SOLVETIME_FACTOR, MTP_WINDOW,
    TARGET_BLOCK_TIME_SECS,
};

/// Compute the difficulty for the block that follows `history`.
///
/// `history` is `(timestamp, difficulty)` in ascending height order. Only the
/// last `DIFFICULTY_WINDOW + 1` entries are consulted.
pub fn next_difficulty(history: &[(u64, u64)], initial: u64, min_difficulty: u64) -> u64 {
    let n = DIFFICULTY_WINDOW as usize;

    // Not enough history yet: hold the starting difficulty.
    if history.len() < n + 1 {
        return initial.max(min_difficulty);
    }

    let window = &history[history.len() - (n + 1)..];
    let target = TARGET_BLOCK_TIME_SECS as i64;
    let min_st = LWMA_MIN_SOLVETIME_FACTOR * target;
    let max_st = LWMA_MAX_SOLVETIME_FACTOR * target;

    let mut weighted_solvetime: i64 = 0;
    let mut difficulty_sum: u128 = 0;

    for i in 1..=n {
        let prev_ts = window[i - 1].0 as i64;
        let this_ts = window[i].0 as i64;
        // Timestamps are only loosely ordered (MTP allows some backwards
        // movement), so solve times are clamped in both directions. An
        // unclamped negative solve time is the classic time-warp lever.
        let solvetime = (this_ts - prev_ts).clamp(min_st, max_st);
        weighted_solvetime += solvetime * (i as i64);
        difficulty_sum += window[i].1 as u128;
    }

    // k = N(N+1)T/2 — the weighted solve time a perfectly paced chain produces.
    let k = (n as i64) * (n as i64 + 1) * target / 2;

    // Floor the denominator so a burst of near-zero solve times cannot send
    // difficulty to infinity (and, via the reciprocal, cannot stall the chain).
    let min_weighted = k / 10;
    if weighted_solvetime < min_weighted {
        weighted_solvetime = min_weighted;
    }

    let avg_difficulty = difficulty_sum / (n as u128);
    let next = avg_difficulty
        .saturating_mul(k as u128)
        .checked_div(weighted_solvetime.max(1) as u128)
        .unwrap_or(avg_difficulty);

    // Clamp per-block movement to ±2× so a single manipulated timestamp cannot
    // produce a wild swing.
    let upper = avg_difficulty.saturating_mul(2);
    let lower = (avg_difficulty / 2).max(1);
    let clamped = next.clamp(lower, upper);

    // The network floor wins over everything. Without this, a hashrate collapse
    // followed by a slow-block window would let difficulty drift low enough
    // that rewriting history becomes cheap.
    (clamped.min(u64::MAX as u128) as u64).max(min_difficulty)
}

/// Median of the last `MTP_WINDOW` timestamps.
///
/// A new block's timestamp must be **strictly greater** than this. That single
/// rule removes the ability to drag timestamps backwards to game the retarget,
/// which v4's "±2 h either way" check permitted freely.
pub fn median_time_past(timestamps: &[u64]) -> u64 {
    if timestamps.is_empty() {
        return 0;
    }
    let take = MTP_WINDOW.min(timestamps.len());
    let mut window: Vec<u64> = timestamps[timestamps.len() - take..].to_vec();
    window.sort_unstable();
    window[window.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = TARGET_BLOCK_TIME_SECS;
    const N: usize = DIFFICULTY_WINDOW as usize;

    fn history(solvetime: u64, difficulty: u64) -> Vec<(u64, u64)> {
        (0..=N as u64)
            .map(|i| (1_000_000 + i * solvetime, difficulty))
            .collect()
    }

    #[test]
    fn on_target_holds_difficulty_steady() {
        let h = history(T, 1_000_000);
        let d = next_difficulty(&h, 1_000_000, 1);
        // Allow a percent of rounding slack.
        assert!(d > 990_000 && d < 1_010_000, "expected ≈1_000_000, got {d}");
    }

    #[test]
    fn fast_blocks_raise_difficulty() {
        let h = history(T / 3, 1_000_000);
        assert!(
            next_difficulty(&h, 1_000_000, 1) > 1_000_000,
            "difficulty must rise when blocks come too fast"
        );
    }

    #[test]
    fn slow_blocks_lower_difficulty() {
        let h = history(T * 4, 1_000_000);
        assert!(
            next_difficulty(&h, 1_000_000, 1) < 1_000_000,
            "difficulty must fall when blocks come too slowly"
        );
    }

    #[test]
    fn respects_the_floor() {
        let h = history(T * 100, 2_000);
        assert!(next_difficulty(&h, 2_000, 5_000) >= 5_000);
    }

    #[test]
    fn single_block_movement_is_bounded() {
        // Even an absurd timestamp gap cannot move difficulty more than 2×.
        let mut h = history(T, 1_000_000);
        let last = h.len() - 1;
        h[last].0 = h[last - 1].0 + 1_000_000;
        let d = next_difficulty(&h, 1_000_000, 1);
        assert!((500_000..=2_000_000).contains(&d), "unbounded swing: {d}");
    }

    #[test]
    fn timewarp_with_backwards_timestamps_cannot_crater_difficulty() {
        // Attacker submits timestamps that go backwards to fake huge solve
        // times. Clamping at −5T bounds the damage.
        let mut h = history(T, 1_000_000);
        for i in 1..h.len() {
            h[i].0 = h[0].0; // every block claims the same instant
        }
        let d = next_difficulty(&h, 1_000_000, 1);
        assert!(
            d >= 500_000,
            "difficulty must not collapse from timestamp games: {d}"
        );
    }

    #[test]
    fn insufficient_history_uses_initial() {
        assert_eq!(next_difficulty(&[(0, 5)], 12_345, 1), 12_345);
    }

    #[test]
    fn mtp_is_the_median_not_the_last() {
        let ts = vec![100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100];
        assert_eq!(median_time_past(&ts), 600);
    }

    #[test]
    fn mtp_ignores_a_single_outlier() {
        let mut ts = vec![100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100];
        ts.push(u64::MAX / 2); // one absurd timestamp
        let m = median_time_past(&ts);
        assert!(m < 2000, "one outlier must not move the median much: {m}");
    }

    #[test]
    fn mtp_handles_short_chains() {
        assert_eq!(median_time_past(&[]), 0);
        assert_eq!(median_time_past(&[42]), 42);
    }
}
