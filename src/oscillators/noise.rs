#[derive(Clone, Copy, Debug)]
pub(crate) struct NoiseState {
    rng: u64,
    clock: f32,
    held: [f32; 2],
    smoothed: [f32; 2],
    burst: [f32; 2],
    low: [f32; 2],
    previous: [f32; 2],
}

impl Default for NoiseState {
    fn default() -> Self {
        Self {
            rng: 1,
            clock: 0.0,
            held: [0.0; 2],
            smoothed: [0.0; 2],
            burst: [0.0; 2],
            low: [0.0; 2],
            previous: [0.0; 2],
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
        texture: f32,
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
                white[1] += mono.mul_add(mono_mix, independent * stereo)
                    * stereo_norm
                    * right_gains[lane];
            }
        }

        let texture = texture.clamp(0.0, 1.0);
        let sparse = smoothstep((texture - 0.5).max(0.0) * 2.0);
        let clock_step = reference_step.clamp(0.0, 0.45) * sparse.mul_add(-31.875, 32.0);
        self.clock += clock_step;
        if self.clock >= 1.0 {
            self.clock -= self.clock.floor();
            let levels = (texture.mul_add(-30.0, 32.0)).round().max(2.0);
            self.held = white.map(|sample| (sample * levels).round() / levels);
            self.burst = self.held;
        }
        for channel in 0..2 {
            self.smoothed[channel] += (self.held[channel] - self.smoothed[channel]) * 0.35;
            self.burst[channel] *= 0.42;
        }
        let held = smoothstep((texture * 2.0).min(1.0));
        let mut output = [0.0; 2];
        for channel in 0..2 {
            let clocked = white[channel].mul_add(1.0 - held, self.smoothed[channel] * held);
            output[channel] = clocked.mul_add(1.0 - sparse, self.burst[channel] * sparse);
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
