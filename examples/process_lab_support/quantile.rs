/// Nearest-rank percentile index for a nonempty sorted sample set.
/// `permille` is in 1..=1000. Split multiplication avoids usize overflow.
pub fn nearest_rank_index(count: usize, permille: usize) -> usize {
    assert!(count > 0 && (1..=1000).contains(&permille));
    (count / 1000 * permille + (count % 1000 * permille).div_ceil(1000)) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_empirical_ranks_include_small_sample_boundaries() {
        for (count, percentile, expected) in [
            (1, 500, 0),
            (20, 950, 18),
            (20, 999, 19),
            (100, 950, 94),
            (1000, 999, 998),
            (3, 500, 1),
        ] {
            assert_eq!(nearest_rank_index(count, percentile), expected);
        }
    }

    #[test]
    fn ranks_are_monotonic_bounded_and_overflow_safe() {
        for count in [1, 2, 20, 999, 1000, 12345, usize::MAX] {
            let mut previous = 0;
            for percentile in 1..=1000 {
                let rank = nearest_rank_index(count, percentile);
                assert!(rank >= previous && rank < count);
                previous = rank;
            }
            assert_eq!(previous, count - 1);
        }
    }
}
