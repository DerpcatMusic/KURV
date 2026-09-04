//! Experimental 1x saw renderer: delayed event BLEP + phase-knot BLAMP.
//! Not wired to production. Input is *unwrapped*, piecewise-linear phase,
//! measured in cycles; callers must preserve signed motion, not shortest-arc unwrap.
//! Kernel construction belongs off the audio thread. Processing allocates nothing.
//! Latency is 17 samples. A 16-crossing cap bounds each process call.

pub const RADIUS: usize = 16;
pub const LATENCY: usize = RADIUS + 1;
pub const MAX_CROSSINGS: usize = 16;
const RING: usize = 64;
const TABLE: usize = 4096;

pub struct EventKernel {
    step: [f64; TABLE + 1],
    ramp: [f64; TABLE + 1],
    knot: [f64; 2 * RADIUS + 1],
}
impl EventKernel {
    /// Blackman-windowed sinc, cutoff 0.45 cycles/sample (guard band is intentional).
    pub fn new() -> Self {
        let mut impulse = [0.0; TABLE + 1];
        let dt = 2.0 * RADIUS as f64 / TABLE as f64;
        for (i, h) in impulse.iter_mut().enumerate() {
            let x = i as f64 * dt - RADIUS as f64;
            let a = std::f64::consts::PI * x / RADIUS as f64;
            let window = 0.42 + 0.5 * a.cos() + 0.08 * (2.0 * a).cos();
            let sinc = if x.abs() < 1e-12 {
                0.9
            } else {
                (std::f64::consts::TAU * 0.45 * x).sin() / (std::f64::consts::PI * x)
            };
            *h = sinc * window;
        }
        let norm = impulse.iter().sum::<f64>() * dt;
        let mut step = [0.0; TABLE + 1];
        let mut ramp = [0.0; TABLE + 1];
        for i in 1..=TABLE {
            step[i] = step[i - 1] + (impulse[i - 1] + impulse[i]) * (0.5 * dt / norm);
            ramp[i] = ramp[i - 1] + (step[i - 1] + step[i]) * (0.5 * dt);
        }
        let mut knot = [0.0; 2 * RADIUS + 1];
        for (i, value) in knot.iter_mut().enumerate() {
            let x = i as f64 - RADIUS as f64;
            *value = ramp[i * TABLE / (2 * RADIUS)] - x.max(0.0);
        }
        Self { step, ramp, knot }
    }
    #[inline(always)]
    fn residuals(&self, x: f64) -> (f64, f64) {
        if x <= -(RADIUS as f64) || x >= RADIUS as f64 {
            return (0.0, 0.0);
        }
        let p = (x + RADIUS as f64) * (TABLE as f64 / (2 * RADIUS) as f64);
        let i = p as usize;
        let f = p - i as f64;
        let s = self.step[i] + f * (self.step[i + 1] - self.step[i]);
        let r = self.ramp[i] + f * (self.ramp[i + 1] - self.ramp[i]);
        (s - if x >= 0.0 { 1.0 } else { 0.0 }, r - x.max(0.0))
    }
}

pub struct EventSaw {
    correction: [f64; RING],
    raw: [f64; RING],
    cursor: usize,
    previous: f64,
    previous_delta: f64,
    initialized: bool,
    slope_valid: bool,
    pub capped_intervals: u64,
}
impl EventSaw {
    pub fn new() -> Self {
        Self {
            correction: [0.0; RING],
            raw: [0.0; RING],
            cursor: 0,
            previous: 0.0,
            previous_delta: 0.0,
            initialized: false,
            slope_valid: false,
            capped_intervals: 0,
        }
    }
    #[inline]
    fn insert(&mut self, kernel: &EventKernel, fraction: f64, jump: f64) {
        // Event at n-1+fraction, output timestamp n-LATENCY.
        for offset in 0..=2 * RADIUS + 1 {
            let elapsed = offset as f64 - RADIUS as f64 - fraction;
            let (blep, _) = kernel.residuals(elapsed);
            let slot = (self.cursor + offset) & (RING - 1);
            self.correction[slot] += jump * blep;
        }
    }
    /// Invalid input is sanitized to zero phase. Over-cap motion emits a bounded
    /// uncorrected interval, counts the degradation, and retains prior residuals.
    /// This fallback is explicitly *not* claimed to be alias-free.
    #[inline]
    pub fn process(&mut self, phase: f64, kernel: &EventKernel) -> f64 {
        let phase = if phase.is_finite() && phase.abs() < 1e12 {
            phase
        } else {
            0.0
        };
        self.raw[(self.cursor + LATENCY) & (RING - 1)] = 2.0 * (phase - phase.floor()) - 1.0;
        if self.initialized {
            let delta = phase - self.previous;
            // At an exactly integral reverse crossing use the right-hand value.
            if delta < 0.0 && self.previous == self.previous.floor() {
                self.raw[(self.cursor + LATENCY - 1) & (RING - 1)] = 1.0;
            }
            let crossings = (phase.floor() - self.previous.floor()).abs();
            if crossings <= MAX_CROSSINGS as f64 {
                // Every input knot changes the derivative of the reconstructed phase.
                let slope = if self.slope_valid {
                    2.0 * (delta - self.previous_delta)
                } else {
                    0.0
                };
                if slope.abs() > 1e-14 {
                    // Knot events are sample-aligned: no fractional lookup is needed.
                    for (offset, &coefficient) in kernel.knot.iter().enumerate() {
                        self.correction[(self.cursor + offset) & (RING - 1)] += slope * coefficient;
                    }
                }
                if delta > 0.0 {
                    let mut edge = self.previous.floor() + 1.0;
                    while edge <= phase {
                        self.insert(kernel, (edge - self.previous) / delta, -2.0);
                        edge += 1.0;
                    }
                } else if delta < 0.0 {
                    let mut edge = self.previous.floor();
                    while edge > phase {
                        self.insert(kernel, (edge - self.previous) / delta, 2.0);
                        edge -= 1.0;
                    }
                }
            } else {
                self.capped_intervals += 1;
            }
            self.slope_valid = crossings <= MAX_CROSSINGS as f64;
            self.previous_delta = delta;
        }
        self.initialized = true;
        self.previous = phase;
        let output = self.raw[self.cursor] + self.correction[self.cursor];
        self.raw[self.cursor] = 0.0;
        self.correction[self.cursor] = 0.0;
        self.cursor = (self.cursor + 1) & (RING - 1);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_endpoints_and_symmetry() {
        let k = EventKernel::new();
        assert!((k.step[TABLE] - 1.0).abs() < 1e-12);
        assert!((k.ramp[TABLE] - RADIUS as f64).abs() < 1e-10);
        for i in 1..TABLE {
            assert!((k.step[i] + k.step[TABLE - i] - 1.0).abs() < 1e-12);
        }
    }
    #[test]
    fn bounded_extreme_input() {
        let k = EventKernel::new();
        let mut s = EventSaw::new();
        for n in 0..1000 {
            assert!(s.process(n as f64 * 1e8, &k).is_finite());
        }
        assert_eq!(s.capped_intervals, 999);
    }
    #[test]
    fn integer_translation_invariance() {
        let k = EventKernel::new();
        let mut a = EventSaw::new();
        let mut b = EventSaw::new();
        for n in 0..1000 {
            let p = n as f64 * 0.1 + 0.4 * (n as f64 * 0.5).sin();
            assert!((a.process(p, &k) - b.process(p + 13.0, &k)).abs() < 1e-10);
        }
    }
    #[test]
    fn exact_integer_reverse_crossings() {
        let k = EventKernel::new();
        let mut a = EventSaw::new();
        let mut b = EventSaw::new();
        for n in 0..1000 {
            let p = n as f64 / 8.0;
            let x = a.process(p, &k);
            let y = b.process(-p, &k);
            if n > 64 {
                assert!((x + y).abs() < 1e-10, "n={n} x={x} y={y}");
            }
        }
    }
}
