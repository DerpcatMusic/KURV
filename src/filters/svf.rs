const DEFAULT_SAMPLE_RATE: f32 = 44_100.0;
const MIN_CUTOFF_HZ: f32 = 5.0;
const NYQUIST_GUARD: f32 = 0.495;
const MIN_Q: f32 = 0.1;
const MAX_Q: f32 = 32.0;
const DENORMAL_LIMIT: f32 = 1.0e-20;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FilterMode {
    #[default]
    LowPass,
    BandPass,
    HighPass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterConfig {
    pub mode: FilterMode,
    pub cutoff_hz: f32,
    pub q: f32,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            mode: FilterMode::LowPass,
            cutoff_hz: 20_000.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
        }
    }
}

impl FilterConfig {
    fn sanitized(self, sample_rate: f32) -> Self {
        let maximum_cutoff = sample_rate * NYQUIST_GUARD;
        Self {
            mode: self.mode,
            cutoff_hz: finite_or(self.cutoff_hz, 20_000.0)
                .clamp(MIN_CUTOFF_HZ.min(maximum_cutoff), maximum_cutoff),
            q: finite_or(self.q, std::f32::consts::FRAC_1_SQRT_2).clamp(MIN_Q, MAX_Q),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FilterCoefficients {
    mode: FilterMode,
    damping: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl FilterConfig {
    #[must_use]
    pub(crate) fn coefficients(self, sample_rate: f32) -> FilterCoefficients {
        let sample_rate = sanitize_sample_rate(sample_rate);
        let config = self.sanitized(sample_rate);
        let g = (std::f32::consts::PI * config.cutoff_hz / sample_rate).tan();
        let damping = config.q.recip();
        let a1 = (1.0 + g * (g + damping)).recip();
        let a2 = g * a1;
        FilterCoefficients {
            mode: config.mode,
            damping,
            a1,
            a2,
            a3: g * a2,
        }
    }
}

impl Default for FilterCoefficients {
    fn default() -> Self {
        FilterConfig::default().coefficients(DEFAULT_SAMPLE_RATE)
    }
}

impl FilterCoefficients {
    #[must_use]
    pub(crate) fn interpolate(self, target: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        Self {
            mode: target.mode,
            damping: amount.mul_add(target.damping - self.damping, self.damping),
            a1: amount.mul_add(target.a1 - self.a1, self.a1),
            a2: amount.mul_add(target.a2 - self.a2, self.a2),
            a3: amount.mul_add(target.a3 - self.a3, self.a3),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ChannelState {
    integrator_1: f32,
    integrator_2: f32,
}

/// Stereo topology-preserving-transform state-variable filter.
///
/// Processing is O(1), uses four `f32` state values, and performs no allocation,
/// locking, I/O, syscalls, or dynamic dispatch.
#[derive(Clone, Copy, Debug)]
pub struct StereoTptSvf {
    left: ChannelState,
    right: ChannelState,
}

impl Default for StereoTptSvf {
    fn default() -> Self {
        Self {
            left: ChannelState::default(),
            right: ChannelState::default(),
        }
    }
}

impl StereoTptSvf {
    pub fn reset(&mut self) {
        self.left = ChannelState::default();
        self.right = ChannelState::default();
    }

    #[must_use]
    #[inline]
    pub(crate) fn process(
        &mut self,
        coefficients: FilterCoefficients,
        left: f32,
        right: f32,
    ) -> (f32, f32) {
        (
            process_channel(&mut self.left, finite_or(left, 0.0), coefficients),
            process_channel(&mut self.right, finite_or(right, 0.0), coefficients),
        )
    }
}

#[inline]
fn process_channel(state: &mut ChannelState, input: f32, coefficients: FilterCoefficients) -> f32 {
    let FilterCoefficients {
        mode,
        damping,
        a1,
        a2,
        a3,
    } = coefficients;
    let v3 = input - state.integrator_2;
    let band = a1.mul_add(state.integrator_1, a2 * v3);
    let low = a2.mul_add(state.integrator_1, a3.mul_add(v3, state.integrator_2));
    let high = (-damping).mul_add(band, input - low);

    state.integrator_1 = zap_denormal(2.0f32.mul_add(band, -state.integrator_1));
    state.integrator_2 = zap_denormal(2.0f32.mul_add(low, -state.integrator_2));

    let output = match mode {
        FilterMode::LowPass => low,
        FilterMode::BandPass => band,
        FilterMode::HighPass => high,
    };
    if output.is_finite() {
        output
    } else {
        *state = ChannelState::default();
        0.0
    }
}

#[inline]
fn zap_denormal(value: f32) -> f32 {
    if value.is_finite() && value.abs() >= DENORMAL_LIMIT {
        value
    } else {
        0.0
    }
}

fn sanitize_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate >= 1.0 {
        sample_rate
    } else {
        DEFAULT_SAMPLE_RATE
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
