//! Intentional pool wait budget, measured from job publication.
//!
//! This is not a hard callback deadline: setup, a single DSP chunk, reduction,
//! and serial fallback still take time outside (or beyond) this interval.
use std::time::Duration;

const EXACT_WAIT_CAP: Duration = Duration::from_millis(2);
const GENERIC_WAIT_CAP: Duration = Duration::from_millis(5);

pub(super) fn nominal_wait_budget(
    job_samples: usize,
    sample_rate: f32,
    exact_saw: bool,
) -> Duration {
    let audio_duration = job_samples as f64 / f64::from(sample_rate.max(1.0));
    let cap = if exact_saw {
        EXACT_WAIT_CAP
    } else {
        GENERIC_WAIT_CAP
    };
    Duration::from_secs_f64(audio_duration * 0.75).min(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_jobs_cannot_inherit_multimillisecond_calibration_caps() {
        for exact in [false, true] {
            assert_eq!(
                nominal_wait_budget(64, 48_000.0, exact),
                Duration::from_millis(1)
            );
            assert_eq!(
                nominal_wait_budget(32, 192_000.0, exact),
                Duration::from_micros(125)
            );
        }
    }

    #[test]
    fn budget_is_bounded_by_job_duration_across_rates_and_sizes() {
        for rate in [44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0] {
            for frames in [0, 8, 16, 32, 64, 128, 256, 512] {
                for exact in [false, true] {
                    let budget = nominal_wait_budget(frames, rate, exact);
                    let ceiling = Duration::from_secs_f64(frames as f64 / f64::from(rate) * 0.75);
                    assert!(budget <= ceiling);
                    assert!(
                        budget
                            <= if exact {
                                EXACT_WAIT_CAP
                            } else {
                                GENERIC_WAIT_CAP
                            }
                    );
                }
            }
        }
    }

    #[test]
    fn oversampling_preserves_the_budget_for_the_same_audio_interval() {
        for factor in [1, 2, 3, 4] {
            for exact in [false, true] {
                assert_eq!(
                    nominal_wait_budget(64 * factor, 48_000.0 * factor as f32, exact),
                    nominal_wait_budget(64, 48_000.0, exact)
                );
            }
        }
    }
}
