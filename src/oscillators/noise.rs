use truce_simd::simd::f32x8;

#[derive(Clone, Copy, Debug)]
pub(crate) struct NoiseState {
    rng: u64,
    burst: [f32; 2],
    low: [f32; 2],
    previous: [f32; 2],
    /// Last gain set the unison compensation was solved for, and its answer.
    /// The square root and divide below cost more than the whole rest of the
    /// kernel at low lane counts, and the unison gains are constant for the
    /// length of a block, so they are computed once and then reused.
    /// The moments are keyed by bit pattern rather than value: the comparison
    /// is exact by construction, and a NaN gain would otherwise never compare
    /// equal to itself and would re-solve on every single sample.
    unison_key: [u32; 4],
    unison_gains: [f32; 2],
}

impl Default for NoiseState {
    fn default() -> Self {
        Self {
            rng: 1,
            burst: [0.0; 2],
            low: [0.0; 2],
            previous: [0.0; 2],
            unison_key: [0; 4],
            unison_gains: [1.0; 2],
        }
    }
}

impl NoiseState {
    pub(crate) fn reset(&mut self, seed: u64) {
        *self = Self {
            rng: seed | 1,
            ..Self::default()
        };
    }

    #[inline(always)]
    fn random(&mut self) -> f32 {
        let mut state = self.rng;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.rng = state;
        let bits = state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40;
        (bits as f32).mul_add(1.0 / 8_388_608.0, -1.0)
    }

    #[inline(always)]
    pub(crate) fn next(
        &mut self,
        reference_step: f32,
        color: f32,
        gaps: f32,
        stereo: f32,
        voices: usize,
        left_gains: &[f32],
        right_gains: &[f32],
    ) -> (f32, f32) {
        let stereo = stereo.clamp(0.0, 1.0);
        let mono_mix = 1.0 - stereo;
        let stereo_norm = if stereo <= f32::EPSILON || mono_mix <= f32::EPSILON {
            1.0
        } else {
            mono_mix.mul_add(mono_mix, stereo * stereo).sqrt().recip()
        };
        let mut white = [0.0; 2];
        // Every branch below draws fresh randomness per lane, so the lanes are
        // uncorrelated -- but `left_gains`/`right_gains` are the unison gains,
        // which are built so that *correlated* content sums to the intended
        // level. Summing N uncorrelated lanes through them lands at
        // `sqrt(sum of squares)` rather than `sum`, so with the usual 1/N gains
        // the noise came out 1/sqrt(N) quieter: raising a noise oscillator's
        // unison count from 1 to 16 dropped it by 12 dB while every other
        // engine held its level. `sum / sqrt(sum of squares)` undoes exactly
        // that, and is 1.0 for a single lane, so mono output is unchanged.
        if stereo <= f32::EPSILON {
            for lane in 0..voices {
                let mono = self.random();
                white[0] += mono * left_gains[lane];
                white[1] += mono * right_gains[lane];
            }
        } else if mono_mix <= f32::EPSILON {
            for lane in 0..voices {
                white[0] += self.random() * left_gains[lane];
                white[1] += self.random() * right_gains[lane];
            }
        } else {
            for lane in 0..voices {
                let mono = self.random();
                let independent = self.random();
                white[0] += mono * left_gains[lane];
                white[1] +=
                    mono.mul_add(mono_mix, independent * stereo) * stereo_norm * right_gains[lane];
            }
        }
        // A single lane is always its own compensation, so the common case
        // pays nothing at all.
        if voices > 1 {
            let (left_sum, left_squares) = gain_moments(&left_gains[..voices]);
            let (right_sum, right_squares) = gain_moments(&right_gains[..voices]);
            let key = [
                left_sum.to_bits(),
                left_squares.to_bits(),
                right_sum.to_bits(),
                right_squares.to_bits(),
            ];
            if key != self.unison_key {
                self.unison_key = key;
                self.unison_gains = [
                    unison_noise_gain(left_sum, left_squares),
                    unison_noise_gain(right_sum, right_squares),
                ];
            }
            white[0] *= self.unison_gains[0];
            white[1] *= self.unison_gains[1];
        }

        let gaps = smoothstep(gaps.clamp(0.0, 1.0));
        let density = 1.0 - gaps;
        let density_squared = density * density;
        let rate_step = reference_step.clamp(0.0, 0.45)
            * (density_squared * density_squared).mul_add(17.91, 0.27);
        let decay = gaps.mul_add(0.59, 0.35);
        if gaps > f32::EPSILON && (self.random() + 1.0) * 0.5 < rate_step {
            self.burst = white.map(|sample| (sample * 4.0).round() * 0.25);
        }
        let geiger_gain = gaps.mul_add(gaps * 5.0, 1.0);
        let mix_gain = (1.0 - (gaps.mul_add(2.0, -1.0)).abs()).mul_add(0.414_213_57, 1.0);
        let mut output = [0.0; 2];
        for channel in 0..2 {
            let geiger = self.burst[channel] * geiger_gain;
            self.burst[channel] *= decay;
            output[channel] = white[channel].mul_add(1.0 - gaps, geiger * gaps) * mix_gain;
        }

        let color = color.clamp(0.0, 1.0);
        let dark = ((0.5 - color) * 2.0).max(0.0);
        let bright = ((color - 0.5) * 2.0).max(0.0);
        let low_alpha = dark.mul_add(-0.995, 1.0);
        let bright_gain = bright.mul_add(0.414_213_57, 1.0).recip();
        for channel in 0..2 {
            self.low[channel] += (output[channel] - self.low[channel]) * low_alpha;
            let darkened = output[channel].mul_add(1.0 - dark, self.low[channel] * dark);
            let brightened = (output[channel] - self.previous[channel] * bright) * bright_gain;
            self.previous[channel] = output[channel];
            output[channel] = if color < 0.5 { darkened } else { brightened };
        }
        (output[0], output[1])
    }
}

/// Sum and sum of squares of a gain set.
///
/// Eight independent accumulators rather than one, because a single scalar
/// chain is latency-bound -- one 4-cycle FMA per lane, which at 64 lanes cost
/// more than the whole rest of the kernel put together.
#[inline]
fn gain_moments(gains: &[f32]) -> (f32, f32) {
    let mut sum = f32x8::ZERO;
    let mut squares = f32x8::ZERO;
    let mut chunks = gains.chunks_exact(8);
    for chunk in &mut chunks {
        let mut lanes = [0.0_f32; 8];
        lanes.copy_from_slice(chunk);
        let lanes = f32x8::from(lanes);
        sum += lanes;
        squares = lanes.mul_add(lanes, squares);
    }
    let mut sum = sum.reduce_add();
    let mut squares = squares.reduce_add();
    for &gain in chunks.remainder() {
        sum += gain;
        squares = gain.mul_add(gain, squares);
    }
    (sum, squares)
}

/// Rescales an uncorrelated lane sum onto the level the same gains would have
/// produced from correlated lanes. Returns 1.0 for a single lane and for a
/// degenerate all-zero gain set, so neither case is perturbed.
#[inline]
fn unison_noise_gain(sum: f32, squares: f32) -> f32 {
    if squares <= f32::MIN_POSITIVE {
        return 1.0;
    }
    sum.abs() * squares.sqrt().recip()
}

#[inline(always)]
fn smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roughness(color: f32) -> f64 {
        let mut noise = NoiseState::default();
        noise.reset(42);
        let mut previous = 0.0_f64;
        let mut signal = 0.0_f64;
        let mut difference = 0.0_f64;
        for _ in 0..65_536 {
            let (sample, _) = noise.next(440.0 / 48_000.0, color, 0.0, 0.0, 1, &[1.0], &[1.0]);
            let sample = f64::from(sample);
            signal += sample * sample;
            difference += (sample - previous) * (sample - previous);
            previous = sample;
        }
        difference / signal
    }

    #[test]
    fn mono_source_is_sample_identical_and_finite() {
        let mut noise = NoiseState::default();
        noise.reset(42);
        for _ in 0..16_384 {
            let (left, right) = noise.next(440.0 / 48_000.0, 0.5, 0.0, 0.0, 1, &[1.0], &[1.0]);
            assert_eq!(left, right);
            assert!(left.is_finite());
        }
    }

    #[test]
    fn color_moves_monotonically_from_dark_to_bright() {
        let dark = roughness(0.0);
        let white = roughness(0.5);
        let bright = roughness(1.0);
        assert!(dark < white * 0.1, "dark={dark} white={white}");
        assert!(bright > white * 1.35, "bright={bright} white={white}");
    }

    #[test]
    fn stereo_endpoint_decorrelates_without_nonfinite_output() {
        let mut noise = NoiseState::default();
        noise.reset(42);
        let mut left_energy = 0.0_f64;
        let mut right_energy = 0.0_f64;
        let mut cross = 0.0_f64;
        for _ in 0..65_536 {
            let (left, right) = noise.next(440.0 / 48_000.0, 0.5, 0.0, 1.0, 1, &[1.0], &[1.0]);
            assert!(left.is_finite() && right.is_finite());
            left_energy += f64::from(left) * f64::from(left);
            right_energy += f64::from(right) * f64::from(right);
            cross += f64::from(left) * f64::from(right);
        }
        let correlation = cross / (left_energy * right_energy).sqrt();
        assert!(correlation.abs() < 0.03, "correlation={correlation}");
        assert!((left_energy / right_energy - 1.0).abs() < 0.03);
    }

    /// A noise oscillator's level must not depend on how many unison lanes it
    /// happens to be spread across; only its width should.
    #[test]
    fn unison_lane_count_does_not_change_the_noise_level() {
        fn energy(voices: u16, stereo: f32) -> f64 {
            let lanes = usize::from(voices);
            let gains = vec![f32::from(voices).recip(); lanes];
            let mut noise = NoiseState::default();
            noise.reset(42);
            let mut total = 0.0_f64;
            for _ in 0..131_072 {
                let (left, right) =
                    noise.next(440.0 / 48_000.0, 0.5, 0.0, stereo, lanes, &gains, &gains);
                total +=
                    f64::from(left).mul_add(f64::from(left), f64::from(right) * f64::from(right));
            }
            (total / 131_072.0).sqrt()
        }

        for stereo in [0.0_f32, 0.5, 1.0] {
            let single = energy(1, stereo);
            for voices in [2_u16, 4, 16, 64] {
                let many = energy(voices, stereo);
                let ratio = many / single;
                assert!(
                    (ratio - 1.0).abs() < 0.03,
                    "stereo={stereo} voices={voices} level ratio {ratio}",
                );
            }
        }
    }

    #[test]
    #[ignore = "manual pinned release microbenchmark"]
    fn benchmark_noise_kernel() {
        use std::hint::black_box;
        use std::time::Instant;

        for stereo in [0.0, 1.0] {
            for voices in [1, 8, 64] {
                let mut noise = NoiseState::default();
                noise.reset(42);
                let gains = [0.125_f32; 64];
                let iterations = 2_000_000 / voices;
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(noise.next(
                        440.0 / 48_000.0,
                        0.5,
                        0.4,
                        stereo,
                        voices,
                        &gains,
                        &gains,
                    ));
                }
                let elapsed = started.elapsed().as_nanos() as f64 / iterations as f64;
                eprintln!("noise stereo={stereo:.0} {voices:>2} lanes: {elapsed:.2} ns/sample");
            }
        }
    }
}
