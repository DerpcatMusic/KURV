use std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct ResonantLowpass {
    integrator_one: f32,
    integrator_two: f32,
}

impl Default for ResonantLowpass {
    fn default() -> Self {
        Self {
            integrator_one: 0.0,
            integrator_two: 0.0,
        }
    }
}

impl ResonantLowpass {
    pub fn reset(&mut self) {
        self.integrator_one = 0.0;
        self.integrator_two = 0.0;
    }

    pub fn process(&mut self, input: f32, cutoff_hz: f32, resonance: f32, sample_rate: f32) -> f32 {
        let sample_rate = sample_rate.max(1.0);
        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.45);
        let g = (PI * cutoff / sample_rate).tan();
        let q = 0.5 + resonance.clamp(0.0, 1.0).powi(2) * 15.5;
        let damping = q.recip();
        let coefficient = (1.0 + g * (g + damping)).recip();
        let band = coefficient * (self.integrator_one + g * (input - self.integrator_two));
        let low = self.integrator_two + g * band;
        self.integrator_one = 2.0 * band - self.integrator_one;
        self.integrator_two = 2.0 * low - self.integrator_two;

        if low.is_finite() {
            low
        } else {
            self.reset();
            0.0
        }
    }
}
