//! Editable periodic oscillator curve compiled to fixed realtime coefficients.

#[path = "wave_curve/bandlimit.rs"]
pub(crate) mod bandlimit;
#[cfg(test)]
#[path = "wave_curve/compiler_experiment.rs"]
mod compiler_experiment;

use std::ops::Deref;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};
use truce_simd::simd::{f32x4, f32x8};
#[cfg(not(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    target_feature = "fma"
)))]
use wide::CmpGt;

pub const MAX_WAVE_KNOTS: usize = 16;
const RT_SEGMENTS: usize = 16;
const COEFFICIENTS_PER_SEGMENT: usize = 4;
const RT_VALUES: usize = RT_SEGMENTS * COEFFICIENTS_PER_SEGMENT;
pub(crate) const WAVE_CURVE_RT_VALUES: usize = RT_VALUES;
pub(crate) const MIN_WAVE_KNOTS: usize = 2;
const MIN_SPACING: f32 = 1.0 / DRAW_FIT_SAMPLES as f32;
const DRAW_FIT_SAMPLES: usize = 256;
const DRAW_FIT_TOLERANCE: f32 = 0.0125;
// Each phase-warp stage remains monotonic while its coefficient stays within
// -1..=1. The horizontal and vertical stages compose, so neither needs to
// consume the other's range.
const MAX_HORIZONTAL_CURVE: f32 = 1.0;
const MAX_VERTICAL_CURVE: f32 = 4.0;

const fn coefficient_index(segment: usize, coefficient: usize) -> usize {
    if cfg!(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    )) {
        coefficient * RT_SEGMENTS + segment
    } else {
        segment * COEFFICIENTS_PER_SEGMENT + coefficient
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, State)]
pub struct WaveKnot {
    pub phase: f32,
    pub value: f32,
    /// Shape of the segment leaving this knot. Zero is linear.
    pub curve: f32,
    /// Horizontal offset of that segment's bend handle. Zero is centered and
    /// preserves the legacy curve exactly.
    pub curve_x: f32,
}

#[derive(Clone, Debug, PartialEq, State)]
pub struct WaveCurveData {
    pub knots: Vec<WaveKnot>,
}

impl Default for WaveCurveData {
    fn default() -> Self {
        Self {
            knots: vec![
                WaveKnot {
                    phase: 0.0,
                    value: 0.0,
                    curve: 0.0,
                    curve_x: 0.0,
                },
                WaveKnot {
                    phase: 0.25,
                    value: 1.0,
                    curve: 0.0,
                    curve_x: 0.0,
                },
                WaveKnot {
                    phase: 0.5,
                    value: 0.0,
                    curve: 0.0,
                    curve_x: 0.0,
                },
                WaveKnot {
                    phase: 0.75,
                    value: -1.0,
                    curve: 0.0,
                    curve_x: 0.0,
                },
            ],
        }
    }
}

pub(crate) fn default_lfo_curve() -> WaveCurveData {
    WaveCurveData {
        knots: vec![
            WaveKnot {
                phase: 0.0,
                value: -1.0,
                ..WaveKnot::default()
            },
            WaveKnot {
                phase: 1.0 - 1.0 / 256.0,
                value: 1.0,
                ..WaveKnot::default()
            },
        ],
    }
}

pub(crate) fn default_grain_curve() -> WaveCurveData {
    WaveCurveData {
        knots: vec![
            WaveKnot {
                phase: 0.0,
                value: 0.0,
                ..WaveKnot::default()
            },
            WaveKnot {
                phase: 0.5,
                value: 1.0,
                curve: 0.55,
                ..WaveKnot::default()
            },
            WaveKnot {
                phase: 1.0 - 1.0 / 256.0,
                value: 0.0,
                ..WaveKnot::default()
            },
        ],
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveCurveRt {
    coefficients: [f32; RT_VALUES],
}

impl Default for WaveCurveRt {
    fn default() -> Self {
        static DEFAULT: OnceLock<WaveCurveRt> = OnceLock::new();
        *DEFAULT.get_or_init(|| WaveCurveData::default().compile_rt())
    }
}

impl WaveCurveData {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            knots: sanitize_knots(&self.knots),
        }
    }

    pub fn compile_rt(&self) -> WaveCurveRt {
        let knots = sanitize_knots(&self.knots);
        let source = SourceCurve::compile(&knots);
        let legacy = if knots.len() <= 3 || has_tight_transition(&knots) {
            WaveCurveRt::from_sampled_source(&source)
        } else {
            WaveCurveRt::from_source(&source)
        };
        let candidate = WaveCurveRt::from_shared_slope_source(&source, &knots);
        let selected = if candidate.proves_better_than(legacy, &source, &knots) {
            candidate
        } else {
            legacy
        };
        let regularized = WaveCurveRt::from_jump_regularized_source(&source, &knots);
        if regularized.proves_regularized_better_than(selected, &source, &knots) {
            regularized
        } else {
            selected
        }
    }
}

struct SourceCurve {
    count: usize,
    x0: [f32; MAX_WAVE_KNOTS],
    x1: [f32; MAX_WAVE_KNOTS],
    inverse_width: [f32; MAX_WAVE_KNOTS],
    a: [f32; MAX_WAVE_KNOTS],
    b: [f32; MAX_WAVE_KNOTS],
    c: [f32; MAX_WAVE_KNOTS],
    d: [f32; MAX_WAVE_KNOTS],
    curve: [f32; MAX_WAVE_KNOTS],
    curve_x: [f32; MAX_WAVE_KNOTS],
}

impl SourceCurve {
    fn compile(knots: &[WaveKnot]) -> Self {
        let count = knots.len();
        let mut widths = [0.0; MAX_WAVE_KNOTS];
        for index in 0..count {
            let next = (index + 1) % count;
            let x0 = knots[index].phase;
            let x1 = if next == 0 { 1.0 } else { knots[next].phase };
            widths[index] = (x1 - x0).max(MIN_SPACING);
        }

        let mut result = Self {
            count,
            x0: [0.0; MAX_WAVE_KNOTS],
            x1: [0.0; MAX_WAVE_KNOTS],
            inverse_width: [0.0; MAX_WAVE_KNOTS],
            a: [0.0; MAX_WAVE_KNOTS],
            b: [0.0; MAX_WAVE_KNOTS],
            c: [0.0; MAX_WAVE_KNOTS],
            d: [0.0; MAX_WAVE_KNOTS],
            curve: [0.0; MAX_WAVE_KNOTS],
            curve_x: [0.0; MAX_WAVE_KNOTS],
        };
        for index in 0..count {
            let next = (index + 1) % count;
            let x0 = knots[index].phase;
            let x1 = if next == 0 { 1.0 } else { knots[next].phase };
            let width = widths[index];
            let y0 = knots[index].value;
            let y1 = knots[next].value;
            let (a, b, c) = (0.0, 0.0, y1 - y0);
            result.x0[index] = x0;
            result.x1[index] = x1;
            result.inverse_width[index] = width.recip();
            result.a[index] = a;
            result.b[index] = b;
            result.c[index] = c;
            result.d[index] = y0;
            result.curve[index] = knots[index].curve;
            result.curve_x[index] = knots[index].curve_x;
        }
        result
    }

    fn eval(&self, phase: f64) -> f64 {
        for index in 0..self.count {
            if phase < f64::from(self.x1[index]) {
                let t = (phase - f64::from(self.x0[index])) * f64::from(self.inverse_width[index]);
                let t = shape_segment_progress_f64(
                    t,
                    f64::from(self.curve[index]),
                    f64::from(self.curve_x[index]),
                );
                return f64::from(self.a[index])
                    .mul_add(t, f64::from(self.b[index]))
                    .mul_add(t, f64::from(self.c[index]))
                    .mul_add(t, f64::from(self.d[index]));
            }
        }
        f64::from(self.d[0])
    }

    fn slope(&self, phase: f32, from_left: bool) -> f32 {
        let probe = if from_left {
            (phase - f32::EPSILON).rem_euclid(1.0)
        } else {
            phase.rem_euclid(1.0)
        };
        let index = (0..self.count)
            .find(|&index| probe < self.x1[index])
            .unwrap_or(self.count - 1);
        let width = self.x1[index] - self.x0[index];
        let t = ((probe - self.x0[index]) / width).clamp(0.0, 1.0);
        let curve = self.curve[index];
        let mut value = t - self.curve_x[index] * t * (1.0 - t);
        let mut slope = 1.0 - self.curve_x[index] + 2.0 * self.curve_x[index] * t;
        let direction = curve.signum();
        let magnitude = curve.abs().min(MAX_VERTICAL_CURVE);
        for _ in 0..magnitude.floor() as usize {
            slope *= 1.0 + direction * (1.0 - 2.0 * value);
            value += direction * value * (1.0 - value);
        }
        slope *= 1.0 + direction * magnitude.fract() * (1.0 - 2.0 * value);
        self.c[index] * slope / width
    }
}

#[inline]
fn shape_segment_progress_f64(progress: f64, curve: f64, curve_x: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let warped = progress - curve_x * progress * (1.0 - progress);
    shape_vertical_progress_f64(warped, curve)
}

#[inline]
pub(crate) fn shape_segment_progress(progress: f32, curve: f32, curve_x: f32) -> f32 {
    let warped = progress - curve_x * progress * (1.0 - progress);
    shape_vertical_progress(warped, curve)
}

fn shape_vertical_progress_f64(mut progress: f64, curve: f64) -> f64 {
    let direction = curve.signum();
    let magnitude = curve.abs().min(f64::from(MAX_VERTICAL_CURVE));
    for _ in 0..magnitude.floor() as usize {
        progress += direction * progress * (1.0 - progress);
    }
    progress + direction * magnitude.fract() * progress * (1.0 - progress)
}

fn shape_vertical_progress(mut progress: f32, curve: f32) -> f32 {
    let direction = curve.signum();
    let magnitude = curve.abs().min(MAX_VERTICAL_CURVE);
    for _ in 0..magnitude.floor() as usize {
        progress += direction * progress * (1.0 - progress);
    }
    progress + direction * magnitude.fract() * progress * (1.0 - progress)
}

pub(crate) fn segment_handle_progress(curve_x: f32) -> f32 {
    (curve_x.mul_add(curve_x, 1.0).sqrt() + 1.0 - curve_x).recip()
}

pub(crate) fn curve_x_from_handle_progress(progress: f32) -> f32 {
    (progress - 0.5) / (progress * (1.0 - progress)).max(f32::EPSILON)
}

pub(crate) fn segment_handle_phase(data: &WaveCurveData, index: usize) -> Option<f32> {
    let knot = data.knots.get(index)?;
    let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
    let progress = segment_handle_progress(knot.curve_x);
    Some((end - knot.phase).mul_add(progress, knot.phase))
}

fn has_tight_transition(knots: &[WaveKnot]) -> bool {
    knots.windows(2).any(|pair| {
        pair[1].phase - pair[0].phase <= MIN_SPACING * 1.5
            && (pair[1].value - pair[0].value).abs() > DRAW_FIT_TOLERANCE
    }) || knots
        .first()
        .zip(knots.last())
        .is_some_and(|(first, last)| {
            1.0 - last.phase <= MIN_SPACING * 1.5
                && (first.value - last.value).abs() > DRAW_FIT_TOLERANCE
        })
}

fn range_overshoot(extrema: (f32, f32, usize)) -> f32 {
    (-1.0 - extrema.0).max(extrema.1 - 1.0).max(0.0)
}

impl WaveCurveRt {
    pub const fn zero() -> Self {
        Self {
            coefficients: [0.0; RT_VALUES],
        }
    }

    pub(crate) const fn from_coefficients(coefficients: [f32; RT_VALUES]) -> Self {
        Self { coefficients }
    }

    pub(crate) const fn coefficients(self) -> [f32; RT_VALUES] {
        self.coefficients
    }

    fn from_sampled_source(source: &SourceCurve) -> Self {
        let values = std::array::from_fn::<_, { RT_SEGMENTS + 1 }, _>(|index| {
            source.eval(index as f64 / RT_SEGMENTS as f64) as f32
        });
        let mut coefficients = [0.0; RT_VALUES];
        for index in 0..RT_SEGMENTS {
            coefficients[coefficient_index(index, 2)] = values[index + 1] - values[index];
            coefficients[coefficient_index(index, 3)] = values[index];
        }
        Self { coefficients }
    }

    fn from_source(source: &SourceCurve) -> Self {
        let mut coefficients = [0.0; RT_VALUES];
        for index in 0..RT_SEGMENTS {
            let phase = index as f64 / RT_SEGMENTS as f64;
            let step = 1.0 / RT_SEGMENTS as f64;
            let y0 = source.eval(phase) as f32;
            let y1 = source.eval(phase + step / 3.0) as f32;
            let y2 = source.eval(phase + step * 2.0 / 3.0) as f32;
            let y3 = source.eval(phase + step) as f32;
            let p = y1 - y0;
            let q = y2 - y0;
            let r = y3 - y0;
            coefficients[coefficient_index(index, 0)] = 4.5_f32.mul_add(r, 13.5 * (p - q));
            coefficients[coefficient_index(index, 1)] =
                (-4.5_f32).mul_add(r, (-22.5_f32).mul_add(p, 18.0 * q));
            coefficients[coefficient_index(index, 2)] =
                9.0_f32.mul_add(p, (-4.5_f32).mul_add(q, r));
            coefficients[coefficient_index(index, 3)] = y0;
        }
        Self { coefficients }
    }

    fn from_shared_slope_source(source: &SourceCurve, knots: &[WaveKnot]) -> Self {
        Self::from_least_squares_source(source, knots, None)
    }

    fn from_jump_regularized_source(source: &SourceCurve, knots: &[WaveKnot]) -> Self {
        Self::from_least_squares_source(source, knots, Some(1.0e-6))
    }

    fn from_least_squares_source(
        source: &SourceCurve,
        knots: &[WaveKnot],
        jump_lambda: Option<f64>,
    ) -> Self {
        const ENDPOINTS: usize = RT_SEGMENTS * 2;
        const FIT_SAMPLES: usize = 32;
        let mut parent = std::array::from_fn::<_, ENDPOINTS, _>(|index| index);
        let find = |parent: &[usize; ENDPOINTS], mut index: usize| {
            while parent[index] != index {
                index = parent[index];
            }
            index
        };
        for boundary in 1..RT_SEGMENTS {
            let phase = boundary as f32 / RT_SEGMENTS as f32;
            let hard = knots.iter().any(|knot| {
                (knot.phase - phase).abs() < 1.0e-6
                    && (source.slope(phase, false) - source.slope(phase, true)).abs() > 1.0e-3
            });
            if jump_lambda.is_none() && !hard {
                let left = find(&parent, (boundary - 1) * 2 + 1);
                let right = find(&parent, boundary * 2);
                parent[right] = left;
            }
        }
        let wrap_hard = (source.slope(0.0, false) - source.slope(0.0, true)).abs() > 1.0e-3;
        if jump_lambda.is_none() && !wrap_hard {
            let left = find(&parent, ENDPOINTS - 1);
            let right = find(&parent, 0);
            parent[right] = left;
        }
        let mut variable = [usize::MAX; ENDPOINTS];
        let mut variable_count = 0;
        for endpoint in 0..ENDPOINTS {
            let root = find(&parent, endpoint);
            if variable[root] == usize::MAX {
                variable[root] = variable_count;
                variable_count += 1;
            }
            variable[endpoint] = variable[root];
        }

        let mut normal = [[0.0_f64; ENDPOINTS + 1]; ENDPOINTS];
        for segment in 0..RT_SEGMENTS {
            let start = segment as f64 / RT_SEGMENTS as f64;
            let width = 1.0 / RT_SEGMENTS as f64;
            let y0 = source.eval(start);
            let y1 = source.eval(start + width);
            let left = variable[segment * 2];
            let right = variable[segment * 2 + 1];
            for sample in 1..FIT_SAMPLES {
                let t = sample as f64 / FIT_SAMPLES as f64;
                let t2 = t * t;
                let t3 = t2 * t;
                let features = [width * (t3 - 2.0 * t2 + t), width * (t3 - t2)];
                let target = source.eval(width.mul_add(t, start))
                    - y0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                    - y1 * (-2.0 * t3 + 3.0 * t2);
                for (row, row_feature) in [(left, features[0]), (right, features[1])] {
                    for (column, column_feature) in [(left, features[0]), (right, features[1])] {
                        normal[row][column] += row_feature * column_feature;
                    }
                    normal[row][variable_count] += row_feature * target;
                }
            }
        }
        if let Some(lambda) = jump_lambda {
            let mut penalize = |left: usize, right: usize| {
                normal[left][left] += lambda;
                normal[right][right] += lambda;
                normal[left][right] -= lambda;
                normal[right][left] -= lambda;
            };
            for boundary in 1..RT_SEGMENTS {
                let phase = boundary as f32 / RT_SEGMENTS as f32;
                let hard = knots.iter().any(|knot| {
                    (knot.phase - phase).abs() < 1.0e-6
                        && (source.slope(phase, false) - source.slope(phase, true)).abs() > 1.0e-3
                });
                if !hard {
                    penalize((boundary - 1) * 2 + 1, boundary * 2);
                }
            }
            if !wrap_hard {
                penalize(ENDPOINTS - 1, 0);
            }
        }
        for pivot in 0..variable_count {
            let best = (pivot..variable_count)
                .max_by(|&left, &right| {
                    normal[left][pivot]
                        .abs()
                        .total_cmp(&normal[right][pivot].abs())
                })
                .unwrap_or(pivot);
            normal.swap(pivot, best);
            let scale = normal[pivot][pivot];
            if scale.abs() <= f64::EPSILON {
                continue;
            }
            for column in pivot..=variable_count {
                normal[pivot][column] /= scale;
            }
            for row in 0..variable_count {
                if row == pivot {
                    continue;
                }
                let scale = normal[row][pivot];
                for column in pivot..=variable_count {
                    normal[row][column] -= scale * normal[pivot][column];
                }
            }
        }
        let slopes = std::array::from_fn::<_, ENDPOINTS, _>(|endpoint| {
            normal[variable[endpoint]][variable_count] as f32
        });
        let mut coefficients = [0.0; RT_VALUES];
        for segment in 0..RT_SEGMENTS {
            let phase = segment as f64 / RT_SEGMENTS as f64;
            let step = 1.0 / RT_SEGMENTS as f64;
            let y0 = source.eval(phase) as f32;
            let y1 = source.eval(phase + step) as f32;
            let m0 = slopes[segment * 2] / RT_SEGMENTS as f32;
            let m1 = slopes[segment * 2 + 1] / RT_SEGMENTS as f32;
            for (coefficient, value) in [
                2.0 * y0 - 2.0 * y1 + m0 + m1,
                -3.0 * y0 + 3.0 * y1 - 2.0 * m0 - m1,
                m0,
                y0,
            ]
            .into_iter()
            .enumerate()
            {
                coefficients[coefficient_index(segment, coefficient)] = value;
            }
        }
        Self { coefficients }
    }

    fn proves_better_than(self, legacy: Self, source: &SourceCurve, knots: &[WaveKnot]) -> bool {
        self.proves_better_than_with(legacy, source, knots, 0.5625, false)
    }

    fn proves_regularized_better_than(
        self,
        current: Self,
        source: &SourceCurve,
        knots: &[WaveKnot],
    ) -> bool {
        self.proves_better_than_with(current, source, knots, 0.9801, true)
    }

    fn proves_better_than_with(
        self,
        legacy: Self,
        source: &SourceCurve,
        knots: &[WaveKnot],
        squared_error_ratio: f64,
        require_energy_reduction: bool,
    ) -> bool {
        let mut candidate_squared = 0.0_f64;
        let mut legacy_squared = 0.0_f64;
        let mut candidate_peak = 0.0_f32;
        let mut legacy_peak = 0.0_f32;
        for sample in 0..DRAW_FIT_SAMPLES {
            let phase = sample as f32 / DRAW_FIT_SAMPLES as f32;
            let expected = source.eval(f64::from(phase)) as f32;
            let candidate_error = self.eval_raw(phase) - expected;
            let legacy_error = legacy.eval_raw(phase) - expected;
            candidate_squared += f64::from(candidate_error * candidate_error);
            legacy_squared += f64::from(legacy_error * legacy_error);
            candidate_peak = candidate_peak.max(candidate_error.abs());
            legacy_peak = legacy_peak.max(legacy_error.abs());
        }
        if candidate_squared > legacy_squared * squared_error_ratio
            || candidate_peak > legacy_peak + 1.0e-7
        {
            return false;
        }
        if knots.iter().any(|knot| {
            (self.eval_raw(knot.phase) - knot.value).abs()
                > (legacy.eval_raw(knot.phase) - knot.value).abs() + 1.0e-6
        }) {
            return false;
        }
        let legacy_range = legacy.extrema();
        let candidate_range = self.extrema();
        if candidate_range.2 > legacy_range.2
            || range_overshoot(candidate_range) > range_overshoot(legacy_range) + 1.0e-6
        {
            return false;
        }
        let legacy_jumps = legacy.derivative_jumps(knots);
        let candidate_jumps = self.derivative_jumps(knots);
        candidate_jumps.0
            <= legacy_jumps.0
                + if require_energy_reduction {
                    1.0e-6
                } else {
                    1.0e-3
                }
            && (legacy_jumps.1 <= 1.0e-3 || candidate_jumps.1 >= legacy_jumps.1 * 0.95)
            && (legacy_jumps.2 <= 1.0e-3 || candidate_jumps.2 >= legacy_jumps.2 * 0.95)
            && (!require_energy_reduction
                || self.derivative_event_energy(knots, source)
                    <= legacy.derivative_event_energy(knots, source))
    }

    fn extrema(self) -> (f32, f32, usize) {
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        let mut crossings = 0;
        for segment in 0..RT_SEGMENTS {
            let [a, b, c, d] = std::array::from_fn(|coefficient| {
                self.coefficients[coefficient_index(segment, coefficient)]
            });
            let mut values = [d, a + b + c + d, 0.0, 0.0];
            let mut count = 2;
            let discriminant = 4.0 * b * b - 12.0 * a * c;
            if a.abs() <= f32::EPSILON {
                if b.abs() > f32::EPSILON {
                    let root = -c / (2.0 * b);
                    if (0.0..1.0).contains(&root) {
                        values[count] = a.mul_add(root, b).mul_add(root, c).mul_add(root, d);
                        count += 1;
                    }
                }
            } else if discriminant >= 0.0 {
                let root = discriminant.sqrt();
                for t in [(-2.0 * b - root) / (6.0 * a), (-2.0 * b + root) / (6.0 * a)] {
                    if (0.0..1.0).contains(&t) {
                        values[count] = a.mul_add(t, b).mul_add(t, c).mul_add(t, d);
                        count += 1;
                    }
                }
            }
            for value in &values[..count] {
                minimum = minimum.min(*value);
                maximum = maximum.max(*value);
                crossings += usize::from(!(-1.0..=1.0).contains(value));
            }
        }
        (minimum, maximum, crossings)
    }

    fn derivative_jumps(self, knots: &[WaveKnot]) -> (f32, f32, f32) {
        let coefficients = |segment| {
            std::array::from_fn::<_, 4, _>(|coefficient| {
                self.coefficients[coefficient_index(segment, coefficient)]
            })
        };
        let mut smooth = 0.0_f32;
        let mut hard = 0.0_f32;
        for segment in 1..RT_SEGMENTS {
            let left = coefficients(segment - 1);
            let right = coefficients(segment);
            let jump =
                ((right[2] - (3.0 * left[0] + 2.0 * left[1] + left[2])) * RT_SEGMENTS as f32).abs();
            if knots
                .iter()
                .any(|knot| (knot.phase - segment as f32 / RT_SEGMENTS as f32).abs() < 1.0e-6)
            {
                hard = hard.max(jump);
            } else {
                smooth = smooth.max(jump);
            }
        }
        let left = coefficients(RT_SEGMENTS - 1);
        let right = coefficients(0);
        let wrap =
            ((right[2] - (3.0 * left[0] + 2.0 * left[1] + left[2])) * RT_SEGMENTS as f32).abs();
        (smooth, hard, wrap)
    }

    fn derivative_event_energy(self, knots: &[WaveKnot], source: &SourceCurve) -> f64 {
        let mut energy = 0.0;
        for segment in 1..RT_SEGMENTS {
            let phase = segment as f32 / RT_SEGMENTS as f32;
            if knots.iter().any(|knot| {
                (knot.phase - phase).abs() < 1.0e-6
                    && (source.slope(phase, false) - source.slope(phase, true)).abs() > 1.0e-3
            }) {
                continue;
            }
            let left = std::array::from_fn::<_, 4, _>(|coefficient| {
                self.coefficients[coefficient_index(segment - 1, coefficient)]
            });
            let right = std::array::from_fn::<_, 4, _>(|coefficient| {
                self.coefficients[coefficient_index(segment, coefficient)]
            });
            let jump = (right[2] - (3.0 * left[0] + 2.0 * left[1] + left[2])) * RT_SEGMENTS as f32;
            energy += f64::from(jump * jump);
        }
        energy
    }

    pub fn interpolate(previous: Self, current: Self, mix: f32) -> Self {
        let mix = mix.clamp(0.0, 1.0);
        Self {
            coefficients: std::array::from_fn(|index| {
                (current.coefficients[index] - previous.coefficients[index])
                    .mul_add(mix, previous.coefficients[index])
            }),
        }
    }

    #[inline]
    fn eval_raw(&self, phase: f32) -> f32 {
        let position = phase * RT_SEGMENTS as f32;
        let index = (position as usize).min(RT_SEGMENTS - 1);
        let t = position - index as f32;
        self.coefficients[coefficient_index(index, 0)]
            .mul_add(t, self.coefficients[coefficient_index(index, 1)])
            .mul_add(t, self.coefficients[coefficient_index(index, 2)])
            .mul_add(t, self.coefficients[coefficient_index(index, 3)])
    }

    #[inline]
    pub fn eval(&self, phase: f32) -> f32 {
        self.eval_raw(phase).clamp(-1.0, 1.0)
    }

    #[inline]
    pub fn eval4(&self, phase: f32x4) -> f32x4 {
        #[cfg(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        ))]
        {
            let [a, b, c, d]: [f32; 4] = phase.into();
            let sample: [f32; 8] = self
                .eval8_avx2(f32x8::from([a, b, c, d, 0.0, 0.0, 0.0, 0.0]))
                .into();
            return f32x4::from([sample[0], sample[1], sample[2], sample[3]]);
        }
        #[cfg(not(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        )))]
        {
            let (index, [a, b, c, d]) = self.select4(phase);
            let t = phase.mul_add(f32x4::splat(RT_SEGMENTS as f32), -index);
            a.mul_add(t, b)
                .mul_add(t, c)
                .mul_add(t, d)
                .fast_max(-f32x4::ONE)
                .fast_min(f32x4::ONE)
        }
    }

    #[inline]
    pub fn eval8(&self, phase: f32x8) -> f32x8 {
        #[cfg(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        ))]
        {
            return self.eval8_avx2(phase);
        }
        #[cfg(not(all(
            target_arch = "x86_64",
            target_feature = "avx2",
            target_feature = "fma"
        )))]
        {
            let phase: [f32; 8] = phase.into();
            f32x8::from(phase.map(|phase| {
                let position = phase * RT_SEGMENTS as f32;
                let index = (position as usize).min(RT_SEGMENTS - 1);
                let t = position - index as f32;
                let base = index * COEFFICIENTS_PER_SEGMENT;
                let [a, b, c, d] = std::array::from_fn(|offset| self.coefficients[base + offset]);
                (((a * t + b) * t + c) * t + d).clamp(-1.0, 1.0)
            }))
        }
    }

    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    ))]
    #[inline]
    fn eval8_avx2(&self, phase: f32x8) -> f32x8 {
        use core::arch::x86_64::{
            _mm256_and_si256, _mm256_blendv_ps, _mm256_castsi256_ps, _mm256_cmpgt_epi32,
            _mm256_cvtepi32_ps, _mm256_cvttps_epi32, _mm256_fmadd_ps, _mm256_loadu_ps,
            _mm256_max_epi32, _mm256_min_epi32, _mm256_mul_ps, _mm256_permutevar8x32_ps,
            _mm256_set1_epi32, _mm256_set1_ps, _mm256_storeu_ps, _mm256_sub_ps,
        };

        let phase: [f32; 8] = phase.into();
        let mut output = [0.0; 8];
        // SAFETY: `phase` and `output` are initialized eight-float arrays and every
        // coefficient plane contains sixteen initialized floats. The segment
        // index is clamped before selecting one of its two eight-value banks.
        unsafe {
            let phase = _mm256_loadu_ps(phase.as_ptr());
            let position = _mm256_mul_ps(phase, _mm256_set1_ps(RT_SEGMENTS as f32));
            let segment = _mm256_min_epi32(
                _mm256_max_epi32(_mm256_cvttps_epi32(position), _mm256_set1_epi32(0)),
                _mm256_set1_epi32((RT_SEGMENTS - 1) as i32),
            );
            let t = _mm256_sub_ps(position, _mm256_cvtepi32_ps(segment));
            let coefficients = self.coefficients.as_ptr();
            let bank_index = _mm256_and_si256(segment, _mm256_set1_epi32(7));
            let upper = _mm256_castsi256_ps(_mm256_cmpgt_epi32(segment, _mm256_set1_epi32(7)));
            let select = |plane: usize| {
                let values = coefficients.add(plane * RT_SEGMENTS);
                let lower = _mm256_permutevar8x32_ps(_mm256_loadu_ps(values), bank_index);
                let upper_bank =
                    _mm256_permutevar8x32_ps(_mm256_loadu_ps(values.add(8)), bank_index);
                _mm256_blendv_ps(lower, upper_bank, upper)
            };
            let a = select(0);
            let b = select(1);
            let c = select(2);
            let d = select(3);
            let sample = _mm256_fmadd_ps(_mm256_fmadd_ps(_mm256_fmadd_ps(a, t, b), t, c), t, d);
            _mm256_storeu_ps(output.as_mut_ptr(), sample);
        }
        f32x8::from(output)
            .fast_max(-f32x8::ONE)
            .fast_min(f32x8::ONE)
    }

    #[cfg(not(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        target_feature = "fma"
    )))]
    #[inline]
    fn select4(&self, phase: f32x4) -> (f32x4, [f32x4; COEFFICIENTS_PER_SEGMENT]) {
        let mut index = f32x4::ZERO;
        let mut selected = std::array::from_fn(|coefficient| {
            f32x4::splat(self.coefficients[coefficient_index(0, coefficient)])
        });
        for segment in 1..RT_SEGMENTS {
            let mask = phase.cmp_gt(f32x4::splat(segment as f32 / RT_SEGMENTS as f32));
            index = mask.blend(f32x4::splat(segment as f32), index);
            for coefficient in 0..COEFFICIENTS_PER_SEGMENT {
                selected[coefficient] = mask.blend(
                    f32x4::splat(self.coefficients[coefficient_index(segment, coefficient)]),
                    selected[coefficient],
                );
            }
        }
        (index, selected)
    }
}

struct AtomicWaveCurve {
    generation: AtomicU32,
    words: [AtomicU32; RT_VALUES],
}

impl AtomicWaveCurve {
    fn new(curve: WaveCurveRt) -> Self {
        let result = Self {
            generation: AtomicU32::new(0),
            words: std::array::from_fn(|_| AtomicU32::new(0)),
        };
        result.store(curve);
        result
    }

    fn store(&self, curve: WaveCurveRt) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        for (target, value) in self.words.iter().zip(curve.coefficients) {
            target.store(value.to_bits(), Ordering::Relaxed);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn try_load(&self) -> Option<WaveCurveRt> {
        let before = self.generation.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let coefficients =
            std::array::from_fn(|index| f32::from_bits(self.words[index].load(Ordering::Relaxed)));
        let curve = WaveCurveRt { coefficients };
        (self.generation.load(Ordering::Acquire) == before).then_some(curve)
    }

    fn try_load_after(&self, observed: u32) -> Option<(u32, WaveCurveRt)> {
        let before = self.generation.load(Ordering::Acquire);
        if before == observed || before & 1 != 0 {
            return None;
        }
        let coefficients =
            std::array::from_fn(|index| f32::from_bits(self.words[index].load(Ordering::Relaxed)));
        let curve = WaveCurveRt { coefficients };
        (self.generation.load(Ordering::Acquire) == before).then_some((before, curve))
    }
}

pub struct WaveCurveState {
    data: RwLock<WaveCurveData>,
    rt: AtomicWaveCurve,
}

impl WaveCurveState {
    pub fn new() -> Self {
        Self::with_data(WaveCurveData::default())
    }

    fn with_data(data: WaveCurveData) -> Self {
        Self {
            rt: AtomicWaveCurve::new(data.compile_rt()),
            data: RwLock::new(data),
        }
    }

    pub fn snapshot(&self) -> WaveCurveData {
        self.data
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    #[must_use]
    pub(crate) fn history_generation(&self) -> u32 {
        self.rt.generation.load(Ordering::Acquire)
    }

    pub fn replace(&self, data: WaveCurveData) {
        let data = data.sanitized();
        let rt = data.compile_rt();
        *self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = data;
        self.rt.store(rt);
    }

    pub fn edit<R>(&self, edit: impl FnOnce(&mut WaveCurveData) -> R) -> R {
        let mut data = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = edit(&mut data);
        data.knots = sanitize_knots(&data.knots);
        let rt = data.compile_rt();
        drop(data);
        self.rt.store(rt);
        result
    }

    pub fn try_curve_rt(&self) -> Option<WaveCurveRt> {
        self.rt.try_load()
    }

    pub fn try_curve_rt_after(&self, observed: u32) -> Option<(u32, WaveCurveRt)> {
        self.rt.try_load_after(observed)
    }
}

pub struct LfoCurveState(WaveCurveState);

impl Default for LfoCurveState {
    fn default() -> Self {
        Self(WaveCurveState::with_data(default_lfo_curve()))
    }
}

impl Deref for LfoCurveState {
    type Target = WaveCurveState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PersistField for LfoCurveState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        self.0.persist_write(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        self.0.persist_read(cursor);
    }
}

pub struct GrainCurveState(WaveCurveState);

impl Default for GrainCurveState {
    fn default() -> Self {
        Self(WaveCurveState::with_data(default_grain_curve()))
    }
}

impl Deref for GrainCurveState {
    type Target = WaveCurveState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PersistField for GrainCurveState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        self.0.persist_write(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        self.0.persist_read(cursor);
    }
}

impl Default for WaveCurveState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for WaveCurveState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        self.snapshot().write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        if let Some(data) = WaveCurveData::read_field(cursor) {
            self.replace(data);
        }
    }
}

/// Resample one endpoint-exclusive periodic cycle and fit it into KURV's
/// editable, maximum-16-knot curve representation. Values outside the editor's
/// bipolar range are scaled together instead of hard-clipped, preserving the
/// cycle's shape and phase. Allocation and fitting are editor/worker-thread work.
#[must_use]
pub fn fit_periodic_samples(samples: &[f32]) -> WaveCurveData {
    if samples.is_empty() || samples.iter().any(|sample| !sample.is_finite()) {
        return WaveCurveData::default();
    }
    let peak = samples
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    let scale = if peak > 1.0 { peak.recip() } else { 1.0 };
    let stroke = (0..=DRAW_FIT_SAMPLES)
        .map(|index| {
            let phase = index as f32 / DRAW_FIT_SAMPLES as f32;
            let position = phase * samples.len() as f32;
            let first = position.floor() as usize % samples.len();
            let next = (first + 1) % samples.len();
            let mix = position - position.floor();
            let value = (samples[next] - samples[first]).mul_add(mix, samples[first]) * scale;
            (phase, value)
        })
        .collect::<Vec<_>>();
    fit_freehand_curve(&WaveCurveData::default(), &stroke)
}

/// Fit a dense UI stroke with the fewest control points that stay inside the
/// visual error budget. Untouched phases come from the existing curve; all
/// fitting and allocation remain on the editor thread.
pub fn fit_freehand_curve(data: &WaveCurveData, stroke: &[(f32, f32)]) -> WaveCurveData {
    if stroke.len() < 2 {
        return data.clone();
    }
    let current = data.compile_rt();
    let mut samples: [f32; DRAW_FIT_SAMPLES] =
        std::array::from_fn(|index| current.eval(index as f32 / DRAW_FIT_SAMPLES as f32));
    for points in stroke.windows(2) {
        let (x0, y0) = points[0];
        let (x1, y1) = points[1];
        let x0 = x0.clamp(0.0, 1.0 - f32::EPSILON);
        let x1 = x1.clamp(0.0, 1.0 - f32::EPSILON);
        let first =
            ((x0.min(x1) * DRAW_FIT_SAMPLES as f32).floor() as usize).min(DRAW_FIT_SAMPLES - 1);
        let last =
            ((x0.max(x1) * DRAW_FIT_SAMPLES as f32).ceil() as usize).min(DRAW_FIT_SAMPLES - 1);
        for (index, sample) in samples.iter_mut().enumerate().take(last + 1).skip(first) {
            let phase = index as f32 / DRAW_FIT_SAMPLES as f32;
            let mix = if (x1 - x0).abs() <= f32::EPSILON {
                1.0
            } else {
                ((phase - x0) / (x1 - x0)).clamp(0.0, 1.0)
            };
            *sample = (y1 - y0).mul_add(mix, y0).clamp(-1.0, 1.0);
        }
    }
    let mut selected = [false; DRAW_FIT_SAMPLES + 1];
    selected[0] = true;
    selected[DRAW_FIT_SAMPLES] = true;
    let mut selected_count = 2;
    while selected_count < MAX_WAVE_KNOTS + 1 {
        let mut best = None;
        let mut start = 0;
        while start < DRAW_FIT_SAMPLES {
            let end = (start + 1..=DRAW_FIT_SAMPLES)
                .find(|&index| selected[index])
                .unwrap_or(DRAW_FIT_SAMPLES);
            let y0 = samples[start % DRAW_FIT_SAMPLES];
            let y1 = samples[end % DRAW_FIT_SAMPLES];
            for index in start + 1..end {
                let mix = (index - start) as f32 / (end - start) as f32;
                let error = (samples[index] - (y1 - y0).mul_add(mix, y0)).abs();
                if best.is_none_or(|(_, best_error)| error > best_error) {
                    best = Some((index, error));
                }
            }
            start = end;
        }
        let Some((index, error)) = best else {
            break;
        };
        if error <= DRAW_FIT_TOLERANCE {
            break;
        }
        selected[index] = true;
        selected_count += 1;
    }
    if selected[..DRAW_FIT_SAMPLES]
        .iter()
        .filter(|selected| **selected)
        .count()
        < MIN_WAVE_KNOTS
    {
        selected[DRAW_FIT_SAMPLES / 2] = true;
    }
    let knots = (0..DRAW_FIT_SAMPLES)
        .filter(|&index| selected[index])
        .map(|index| WaveKnot {
            phase: index as f32 / DRAW_FIT_SAMPLES as f32,
            value: samples[index],
            curve: 0.0,
            curve_x: 0.0,
        })
        .collect::<Vec<_>>();
    WaveCurveData {
        knots: sanitize_knots(&knots),
    }
}

pub fn insert_knot(data: &mut WaveCurveData, phase: f32, value: f32) -> bool {
    if data.knots.len() >= MAX_WAVE_KNOTS {
        return false;
    }
    let phase = phase.clamp(MIN_SPACING, 1.0 - MIN_SPACING);
    if data
        .knots
        .iter()
        .any(|knot| (knot.phase - phase).abs() < MIN_SPACING)
    {
        return false;
    }
    data.knots.push(WaveKnot {
        phase,
        value: value.clamp(-1.0, 1.0),
        curve: 0.0,
        curve_x: 0.0,
    });
    data.knots.sort_by(|a, b| a.phase.total_cmp(&b.phase));
    true
}

pub fn move_knot(data: &mut WaveCurveData, index: usize, phase: f32, value: f32) -> bool {
    if index >= data.knots.len() {
        return false;
    }
    let minimum = if index == 0 {
        0.0
    } else {
        data.knots[index - 1].phase + MIN_SPACING
    };
    let maximum = if index + 1 == data.knots.len() {
        1.0 - MIN_SPACING
    } else {
        data.knots[index + 1].phase - MIN_SPACING
    };
    data.knots[index].phase = if index == 0 {
        0.0
    } else {
        phase.clamp(minimum, maximum)
    };
    data.knots[index].value = value.clamp(-1.0, 1.0);
    true
}

pub fn remove_knot(data: &mut WaveCurveData, index: usize) -> bool {
    if index == 0 || data.knots.len() <= MIN_WAVE_KNOTS || index >= data.knots.len() {
        return false;
    }
    data.knots.remove(index);
    true
}

pub fn set_segment_bend(data: &mut WaveCurveData, index: usize, curve: f32, curve_x: f32) -> bool {
    let Some(knot) = data.knots.get_mut(index) else {
        return false;
    };
    knot.curve = curve.clamp(-MAX_VERTICAL_CURVE, MAX_VERTICAL_CURVE);
    knot.curve_x = curve_x.clamp(-MAX_HORIZONTAL_CURVE, MAX_HORIZONTAL_CURVE);
    true
}

fn sanitize_knots(knots: &[WaveKnot]) -> Vec<WaveKnot> {
    let mut result = knots
        .iter()
        .copied()
        .filter(|knot| knot.phase.is_finite() && knot.value.is_finite())
        .map(|mut knot| {
            knot.phase = knot.phase.clamp(0.0, 1.0 - MIN_SPACING);
            knot.value = knot.value.clamp(-1.0, 1.0);
            knot.curve = if knot.curve.is_finite() {
                knot.curve.clamp(-MAX_VERTICAL_CURVE, MAX_VERTICAL_CURVE)
            } else {
                0.0
            };
            knot.curve_x = if knot.curve_x.is_finite() {
                knot.curve_x
                    .clamp(-MAX_HORIZONTAL_CURVE, MAX_HORIZONTAL_CURVE)
            } else {
                0.0
            };
            knot
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| a.phase.total_cmp(&b.phase));
    result.dedup_by(|left, right| (left.phase - right.phase).abs() < MIN_SPACING);
    result.truncate(MAX_WAVE_KNOTS);
    if result.len() < MIN_WAVE_KNOTS {
        return WaveCurveData::default().knots;
    }
    result[0].phase = 0.0;
    result
}

#[cfg(test)]
mod topology_tests {
    use super::{DRAW_FIT_SAMPLES, SourceCurve, WaveCurveData, WaveKnot, fit_periodic_samples};

    #[test]
    fn neutral_multi_knot_segments_are_linear() {
        let data = WaveCurveData {
            knots: vec![
                WaveKnot {
                    phase: 0.0,
                    value: -1.0,
                    curve: 0.0,
                    curve_x: 0.0,
                },
                WaveKnot {
                    phase: 0.2,
                    value: 0.6,
                    curve: 0.0,
                    curve_x: 0.0,
                },
                WaveKnot {
                    phase: 0.55,
                    value: 0.9,
                    curve: 0.0,
                    curve_x: 0.0,
                },
                WaveKnot {
                    phase: 0.8,
                    value: -0.4,
                    curve: 0.0,
                    curve_x: 0.0,
                },
            ],
        };
        let source = SourceCurve::compile(&data.knots);
        assert!((source.eval(0.1) + 0.2).abs() < 1.0e-6);
        assert!((source.eval(0.375) - 0.75).abs() < 1.0e-6);
        assert!((source.eval(0.675) - 0.25).abs() < 1.0e-6);
        let rt = data.compile_rt();
        let max_error = (0..4_096)
            .map(|index| {
                let phase = index as f32 / 4_096.0;
                (rt.eval(phase) - source.eval(f64::from(phase)) as f32).abs()
            })
            .fold(0.0_f32, f32::max);
        assert!(max_error < 0.025, "max realtime error={max_error}");
    }

    #[test]
    fn ideal_saw_uses_two_corner_knots_without_rt_overshoot() {
        let samples = (0..DRAW_FIT_SAMPLES)
            .map(|index| index as f32 / DRAW_FIT_SAMPLES as f32 * 2.0 - 1.0)
            .collect::<Vec<_>>();
        let data = fit_periodic_samples(&samples);
        assert_eq!(data.knots.len(), 2, "{:#?}", data.knots);
        assert_eq!(data.knots[0].phase, 0.0);
        assert!(data.knots[1].phase >= 1.0 - 1.0 / DRAW_FIT_SAMPLES as f32);
        let curve = data.compile_rt();
        for index in 0..DRAW_FIT_SAMPLES {
            let value = curve.eval(index as f32 / DRAW_FIT_SAMPLES as f32);
            assert!(value.is_finite());
            assert!(
                (-1.000_001..=1.000_001).contains(&value),
                "{index}: {value}"
            );
        }
    }

    #[test]
    fn ideal_triangle_uses_only_its_two_turning_points() {
        let samples = (0..DRAW_FIT_SAMPLES)
            .map(|index| {
                let phase = index as f32 / DRAW_FIT_SAMPLES as f32;
                if phase < 0.5 {
                    phase * 4.0 - 1.0
                } else {
                    3.0 - phase * 4.0
                }
            })
            .collect::<Vec<_>>();
        let data = fit_periodic_samples(&samples);
        assert_eq!(data.knots.len(), 2, "{:#?}", data.knots);
    }
}

#[cfg(test)]
mod exact_fourier_experiment {
    use std::hint::black_box;
    use std::time::Instant;

    use crate::dsp::Complex;
    use crate::oversampling::{LATENCY_SAMPLES, StereoOversampler};

    use super::bandlimit::BandlimitedWaveCurve;
    use super::{COEFFICIENTS_PER_SEGMENT, RT_SEGMENTS, WaveCurveRt, coefficient_index};

    const MAX_PARTIALS: usize = 16;
    const SAMPLE_RATE: f64 = 48_000.0;

    fn exact_coefficients(curve: WaveCurveRt, partials: usize) -> [Complex; MAX_PARTIALS + 1] {
        let coefficients = curve.coefficients();
        std::array::from_fn(|harmonic| {
            if harmonic > partials {
                return Complex::ZERO;
            }
            if harmonic == 0 {
                return (0..RT_SEGMENTS)
                    .map(|segment| {
                        let [a, b, c, d] =
                            std::array::from_fn::<_, COEFFICIENTS_PER_SEGMENT, _>(|coefficient| {
                                f64::from(coefficients[coefficient_index(segment, coefficient)])
                            });
                        (a / 4.0 + b / 3.0 + c / 2.0 + d) / RT_SEGMENTS as f64
                    })
                    .sum::<f64>()
                    .into();
            }

            let omega = std::f64::consts::TAU * harmonic as f64 / RT_SEGMENTS as f64;
            let endpoint = Complex::from_polar(1.0, -omega);
            let inverse_i_omega = Complex::new(0.0, -omega.recip());
            let mut integrals = [Complex::ZERO; COEFFICIENTS_PER_SEGMENT];
            integrals[0] = (Complex::new(1.0, 0.0) - endpoint) * inverse_i_omega;
            for order in 1..COEFFICIENTS_PER_SEGMENT {
                integrals[order] = -endpoint * inverse_i_omega
                    + integrals[order - 1] * inverse_i_omega * order as f64;
            }

            (0..RT_SEGMENTS)
                .map(|segment| {
                    let [a, b, c, d] =
                        std::array::from_fn::<_, COEFFICIENTS_PER_SEGMENT, _>(|coefficient| {
                            f64::from(coefficients[coefficient_index(segment, coefficient)])
                        });
                    let segment_phase = Complex::from_polar(
                        1.0,
                        -std::f64::consts::TAU * harmonic as f64 * segment as f64
                            / RT_SEGMENTS as f64,
                    );
                    segment_phase
                        * (a * integrals[3]
                            + b * integrals[2]
                            + c * integrals[1]
                            + d * integrals[0])
                        / RT_SEGMENTS as f64
                })
                .sum()
        })
    }

    fn saw_curve() -> WaveCurveRt {
        let mut coefficients = [0.0; RT_SEGMENTS * COEFFICIENTS_PER_SEGMENT];
        for segment in 0..RT_SEGMENTS {
            coefficients[coefficient_index(segment, 2)] = 2.0 / RT_SEGMENTS as f32;
            coefficients[coefficient_index(segment, 3)] =
                -1.0 + 2.0 * segment as f32 / RT_SEGMENTS as f32;
        }
        WaveCurveRt::from_coefficients(coefficients)
    }

    fn square_curve() -> WaveCurveRt {
        let mut coefficients = [0.0; RT_SEGMENTS * COEFFICIENTS_PER_SEGMENT];
        for segment in 0..RT_SEGMENTS {
            coefficients[coefficient_index(segment, 3)] = if segment < 8 { 1.0 } else { -1.0 };
        }
        WaveCurveRt::from_coefficients(coefficients)
    }

    fn exact_sample(
        coefficients: &[Complex; MAX_PARTIALS + 1],
        partials: usize,
        phase: f64,
    ) -> f64 {
        coefficients[0].re
            + 2.0
                * (1..=partials)
                    .map(|harmonic| {
                        let rotation = Complex::from_polar(
                            1.0,
                            std::f64::consts::TAU * harmonic as f64 * phase,
                        );
                        (coefficients[harmonic] * rotation).re
                    })
                    .sum::<f64>()
    }

    struct Additive {
        count: usize,
        dc: f32,
        coefficient_re: [f32; MAX_PARTIALS],
        coefficient_im: [f32; MAX_PARTIALS],
        phase_re: [f32; MAX_PARTIALS],
        phase_im: [f32; MAX_PARTIALS],
        rotation_re: [f32; MAX_PARTIALS],
        rotation_im: [f32; MAX_PARTIALS],
    }

    impl Additive {
        fn new(coefficients: &[Complex; MAX_PARTIALS + 1], count: usize, phase_step: f32) -> Self {
            let mut result = Self {
                count,
                dc: coefficients[0].re as f32,
                coefficient_re: [0.0; MAX_PARTIALS],
                coefficient_im: [0.0; MAX_PARTIALS],
                phase_re: [1.0; MAX_PARTIALS],
                phase_im: [0.0; MAX_PARTIALS],
                rotation_re: [0.0; MAX_PARTIALS],
                rotation_im: [0.0; MAX_PARTIALS],
            };
            for harmonic in 1..=count {
                let index = harmonic - 1;
                result.coefficient_re[index] = coefficients[harmonic].re as f32;
                result.coefficient_im[index] = coefficients[harmonic].im as f32;
                let angle = std::f32::consts::TAU * harmonic as f32 * phase_step;
                (result.rotation_im[index], result.rotation_re[index]) = angle.sin_cos();
            }
            result
        }

        #[inline]
        fn next(&mut self) -> f32 {
            let mut output = self.dc;
            for index in 0..self.count {
                output += 2.0
                    * (self.coefficient_re[index] * self.phase_re[index]
                        - self.coefficient_im[index] * self.phase_im[index]);
                let re = self.phase_re[index] * self.rotation_re[index]
                    - self.phase_im[index] * self.rotation_im[index];
                self.phase_im[index] = self.phase_re[index] * self.rotation_im[index]
                    + self.phase_im[index] * self.rotation_re[index];
                self.phase_re[index] = re;
            }
            output
        }
    }

    fn rms_error(
        start: usize,
        mut candidate: impl FnMut(usize) -> f64,
        mut reference: impl FnMut(usize) -> f64,
    ) -> f64 {
        for index in 0..start {
            black_box(candidate(index));
        }
        let error = (0..131_072)
            .map(|index| {
                let index = start + index;
                (candidate(index) - reference(index)).powi(2)
            })
            .sum::<f64>()
            / 131_072.0;
        error.sqrt()
    }

    fn median_ns(mut run: impl FnMut()) -> f64 {
        let mut measurements = [0.0; 7];
        for measurement in &mut measurements {
            let started = Instant::now();
            run();
            *measurement = started.elapsed().as_nanos() as f64 / 2_000_000.0;
        }
        measurements.sort_by(f64::total_cmp);
        measurements[measurements.len() / 2]
    }

    #[test]
    fn exact_polynomial_coefficients_reconstruct_a_saw() {
        let coefficients = exact_coefficients(saw_curve(), MAX_PARTIALS);
        for harmonic in 1..=MAX_PARTIALS {
            let expected = 1.0 / (std::f64::consts::PI * harmonic as f64);
            assert!((coefficients[harmonic].im - expected).abs() < 2.0e-12);
            assert!(coefficients[harmonic].re.abs() < 2.0e-12);
        }
    }

    #[test]
    #[ignore = "manual pinned release exact-Fourier experiment"]
    fn benchmark_exact_fourier_high_note_crossover() {
        for (name, curve) in [("saw", saw_curve()), ("square", square_curve())] {
            let spectral = BandlimitedWaveCurve::compile(|phase| curve.eval(phase))
                .expect("finite curve must compile");
            for midi in [93.0_f64, 105.0, 117.0, 123.0] {
                let frequency = 440.0 * 2.0_f64.powf((midi - 69.0) / 12.0);
                let step = (frequency / SAMPLE_RATE) as f32;
                let mip = BandlimitedWaveCurve::mip_for_phase_step(step).expect("playable note");
                let mip_partials = BandlimitedWaveCurve::harmonic_cap(mip).expect("valid mip");
                let partials = ((0.5 / step).ceil() as usize - 1).min(MAX_PARTIALS);
                assert!(partials <= MAX_PARTIALS);
                let coefficients = exact_coefficients(curve, partials);
                let reference = |index: usize| {
                    exact_sample(&coefficients, partials, index as f64 * f64::from(step))
                };

                let analytic_error = rms_error(
                    0,
                    |index| f64::from(curve.eval((index as f32 * step).fract())),
                    reference,
                );
                let spectral_error = rms_error(
                    0,
                    |index| f64::from(spectral.eval_mip(index as f32 * step, mip)),
                    reference,
                );
                let mut additive = Additive::new(&coefficients, partials, step);
                let additive_error = rms_error(0, |_| f64::from(additive.next()), reference);

                let latency = LATENCY_SAMPLES as usize;
                let mut shipping_phase = 0.0_f32;
                let mut shipping_oversampler = StereoOversampler::default();
                let shipping_error = rms_error(
                    latency * 2,
                    |_| {
                        for _ in 0..2 {
                            let sample = curve.eval(shipping_phase);
                            shipping_phase = (shipping_phase + step * 0.5).fract();
                            shipping_oversampler.push(sample, sample);
                        }
                        f64::from(shipping_oversampler.output().0)
                    },
                    |index| {
                        exact_sample(
                            &coefficients,
                            partials,
                            (index as f64 - f64::from(LATENCY_SAMPLES) + 0.5) * f64::from(step),
                        )
                    },
                );

                let mut candidate_additive = Additive::new(&coefficients, partials, step);
                let mut candidate_oversampler = StereoOversampler::default();
                candidate_oversampler.reset(1);
                let candidate_error = rms_error(
                    latency * 2,
                    |_| {
                        let sample = if black_box(partials <= MAX_PARTIALS) {
                            candidate_additive.next()
                        } else {
                            unreachable!()
                        };
                        f64::from(candidate_oversampler.process_direct(sample, sample).0)
                    },
                    |index| reference(index - latency),
                );

                let analytic_ns = median_ns(|| {
                    let mut phase = 0.0_f32;
                    for _ in 0..2_000_000 {
                        black_box(curve.eval(phase));
                        phase = (phase + step).fract();
                    }
                    black_box(phase);
                });
                let spectral_ns = median_ns(|| {
                    let mut phase = 0.0_f32;
                    for _ in 0..2_000_000 {
                        black_box(spectral.eval_mip(phase, mip));
                        phase = (phase + step).fract();
                    }
                    black_box(phase);
                });
                let additive_ns = median_ns(|| {
                    let mut additive = Additive::new(&coefficients, partials, step);
                    for _ in 0..2_000_000 {
                        black_box(additive.next());
                    }
                });
                let shipping_ns = median_ns(|| {
                    let mut phase = 0.0_f32;
                    let mut oversampler = StereoOversampler::default();
                    for _ in 0..2_000_000 {
                        for _ in 0..2 {
                            let sample = curve.eval(phase);
                            phase = (phase + step * 0.5).fract();
                            oversampler.push(sample, sample);
                        }
                        black_box(oversampler.output());
                    }
                });
                let candidate_ns = median_ns(|| {
                    let mut additive = Additive::new(&coefficients, partials, step);
                    let mut oversampler = StereoOversampler::default();
                    oversampler.reset(1);
                    for _ in 0..2_000_000 {
                        let sample = if black_box(partials <= MAX_PARTIALS) {
                            additive.next()
                        } else {
                            unreachable!()
                        };
                        black_box(oversampler.process_direct(sample, sample));
                    }
                });

                eprintln!(
                    "fourier shape={name} midi={midi:.0} hz={frequency:.3} partials={partials} mip_partials={mip_partials} rms analytic1x={analytic_error:.8} spectral1x={spectral_error:.8} additive_raw={additive_error:.8} shipping2x={shipping_error:.8} candidate1x={candidate_error:.8} ns analytic_raw={analytic_ns:.3} spectral_raw={spectral_ns:.3} additive_raw={additive_ns:.3} shipping2x={shipping_ns:.3} candidate1x={candidate_ns:.3}"
                );
            }
        }
    }
}
