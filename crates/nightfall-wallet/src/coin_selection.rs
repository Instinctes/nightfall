//! Bounded largest-first selection, preserving the original stable tie order.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

type Rank = (u64, Reverse<usize>);

/// The iterator supplies ascending wallet indices. Keep only the best `limit`
/// ranks, but accumulate the saturating total of every eligible output.
pub(super) fn bounded_largest(
    values: impl Iterator<Item = (usize, u64)>,
    limit: usize,
) -> (u64, Vec<usize>) {
    let mut total = 0u64;
    let mut heap: BinaryHeap<Reverse<Rank>> = BinaryHeap::new();

    for (index, value) in values {
        total = total.saturating_add(value);

        if limit == 0 {
            continue;
        }

        let rank = (value, Reverse(index));

        if heap.len() < limit {
            heap.push(Reverse(rank));
        } else {
            let mut worst = heap.peek_mut().expect("nonempty heap");

            if rank > worst.0 {
                *worst = Reverse(rank);
            }
        }
    }

    let selected = heap
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse((_, Reverse(index)))| index)
        .collect();

    (total, selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(values: &[(usize, u64)], limit: usize) -> (u64, Vec<usize>) {
        let mut all: Vec<_> = values.iter().collect();

        all.sort_by_key(|v| Reverse(v.1));

        let total = all.iter().fold(0u64, |sum, v| sum.saturating_add(v.1));

        (total, all.into_iter().take(limit).map(|v| v.0).collect())
    }

    fn check(values: &[(usize, u64)], limit: usize) {
        assert_eq!(
            bounded_largest(values.iter().copied(), limit),
            reference(values, limit)
        );
    }

    #[test]
    fn bounds_ties_and_saturation() {
        let values = [(1, 0), (3, 7), (8, 7), (11, u64::MAX), (15, 7)];

        for limit in 0..=7 {
            check(&values, limit);
            check(&[], limit);
        }

        assert_eq!(
            bounded_largest(values.into_iter(), 3),
            (u64::MAX, vec![11, 3, 8])
        );
    }

    #[test]
    fn differential_against_full_stable_sort() {
        let mut seed = 0x9e3779b97f4a7c15u64;

        for n in [0, 1, 2, 31, 511, 512, 513, 25_000] {
            let values: Vec<_> = (0..n)
                .map(|i| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;

                    (i * 3, seed % 113)
                })
                .collect();

            for k in [0, 1, 3, 32, 511, 512, 513, n + 1] {
                check(&values, k);
            }
        }

        for descending in [false, true] {
            let values: Vec<_> = (0..25_000)
                .map(|i| {
                    (
                        i,
                        if descending {
                            (25_000 - i) as u64
                        } else {
                            i as u64
                        },
                    )
                })
                .collect();

            check(&values, 512);
        }
    }

    #[test]
    #[ignore]
    fn synthetic_selection_timing() {
        use std::{hint::black_box, time::Instant};

        for n in [25_000usize, 250_000] {
            let values: Vec<_> = (0..n)
                .map(|i| (i, (i as u64).wrapping_mul(2654435761) % 65521))
                .collect();

            check(&values, 512);

            let mut old = Vec::new();
            let mut new = Vec::new();

            let a = || {
                let t = Instant::now();

                black_box(reference(black_box(&values), 512));

                t.elapsed().as_nanos()
            };

            let b = || {
                let t = Instant::now();

                black_box(bounded_largest(black_box(&values).iter().copied(), 512));

                t.elapsed().as_nanos()
            };

            for i in 0..5 {
                if i % 2 == 0 {
                    old.push(a());
                    new.push(b());
                } else {
                    new.push(b());
                    old.push(a());
                }
            }

            old.sort_unstable();
            new.sort_unstable();

            println!(
                "synthetic n={n} k=512 old_median_ns={} new_median_ns={}",
                old[2], new[2]
            );
        }
    }
}
