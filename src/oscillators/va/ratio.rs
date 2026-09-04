//! Immutable harmonic-prefix tables for ratio-domain VA filtering.

use std::sync::OnceLock;

use super::antialias::{aligned_sine_phase, aligned_sine_phase8, wrap_phase8};
use super::warp::warp_phase8;
use super::{PhaseWarpMode, VaOscillator, shape_morph_gain, warp_phase_scalar};
use truce_simd::simd::{f32x4, f32x8};
use wide::{CmpGt, CmpLt};

const MIN_RATIO: f32 = 0.0;
const MAX_RATIO: f32 = 1_024.0;

const TABLE_SIZE: usize = 2_048;
const TABLE_MASK: usize = TABLE_SIZE - 1;
const MAX_HARMONIC: usize = TABLE_SIZE / 2 - 1;
const LINEAR_HARMONIC_LIMIT: usize = 64;
const ROWS: usize = MAX_HARMONIC + 1;

static PREFIXES: OnceLock<PrefixBank> = OnceLock::new();

struct PrefixBank {
    saw: Box<[f32]>,
    triangle: Box<[f32]>,
}

#[derive(Clone, Copy)]
pub(crate) struct PreparedRatioSource {
    prefixes: &'static PrefixBank,
    segment: usize,
    blend: f32,
    gain: f32,
    pulse_shift: f32,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    first: usize,
    low: usize,
    low_fraction: f32,
    requested_maximum: usize,
    high_fraction: f32,
}

impl PreparedRatioSource {
    #[must_use]
    pub(crate) fn new(
        shape: f32,
        pulse_width: f32,
        warp_mode: PhaseWarpMode,
        warp_amount: f32,
        ratio_band: (f32, f32),
    ) -> Option<Self> {
        let shape = shape.clamp(0.0, 3.0);
        let segment = shape.floor() as usize;
        // Both band edges are continuous controls, but the prefix tables are
        // indexed by whole harmonics. Keeping only the floor makes a partial
        // switch on or off within one sample as an edge sweeps past it, which
        // under cutoff modulation is an audible click per partial. Keep the
        // fractional parts too and let the render path weight the harmonic
        // just outside each edge by them.
        let lower = ratio_band.0.clamp(MIN_RATIO, MAX_RATIO);
        let upper = ratio_band.1.clamp(MIN_RATIO, MAX_RATIO);
        let low = lower.floor();
        let maximum = upper.floor();
        let low_index = low as usize;
        Some(Self {
            prefixes: PREFIXES.get()?,
            segment,
            blend: shape - segment as f32,
            gain: shape_morph_gain(shape),
            pulse_shift: 1.0 - pulse_width.clamp(0.03, 0.97),
            warp_mode,
            warp_amount,
            first: low_index.saturating_add(1),
            low: low_index,
            low_fraction: lower - low,
            requested_maximum: maximum as usize,
            high_fraction: upper - maximum,
        })
    }
}

impl PreparedRatioSource {
    /// Weights for the two harmonics that straddle the band edges.
    ///
    /// Returns `(high, low)`: how much of harmonic `requested_maximum + 1` to
    /// add on top of the integer prefix range, and how much of harmonic
    /// `low + 1` to take back out of it. Both are the fractional parts of the
    /// respective edges, so a partial crossfades in or out over one harmonic
    /// of edge travel instead of switching.
    ///
    /// `maximum` is the Nyquist-clamped harmonic the caller will actually sum
    /// to. Where that clamp binds, the edge weight is dropped: antialiasing
    /// outranks smoothing, and a harmonic the prefix range had to discard must
    /// not come back through the edge term.
    #[inline]
    fn edge_weights(&self, maximum: usize, admits_high: bool) -> (f32, f32) {
        let high = if admits_high { self.high_fraction } else { 0.0 };
        if self.low == self.requested_maximum {
            // The band is narrower than one harmonic, so both ramps land on
            // the same partial and the integer range is empty. Multiplying the
            // ramps keeps that partial bounded by each edge independently and
            // fades it to silence as the band closes, rather than letting the
            // two edge terms cancel into a wrong non-zero weight.
            ((1.0 - self.low_fraction) * high, 0.0)
        } else if self.low < maximum {
            (high, self.low_fraction)
        } else {
            // Nyquist already cut the band below its own lower edge, so the
            // harmonic there was never summed and there is nothing to remove.
            (high, 0.0)
        }
    }
}

impl PreparedRatioSource {
    /// Whether the harmonic just above the band survives antialiasing.
    ///
    /// It does only when the Nyquist clamp left the integer range at the
    /// requested edge *and* the next harmonic up is itself below Nyquist.
    #[inline]
    #[allow(
        clippy::cast_precision_loss,
        reason = "harmonic indices are bounded by MAX_HARMONIC and exact in f32"
    )]
    fn admits_high_edge(&self, maximum: usize, step: f32) -> bool {
        maximum == self.requested_maximum && ((self.requested_maximum + 1) as f32) * step < 0.5
    }
}

impl PreparedRatioSource {
    /// Per-lane summation limit and fractional edge weights for one vector of
    /// phase steps.
    ///
    /// Each lane has its own Nyquist clamp and so its own edge weights, but the
    /// two harmonics those weights apply to are shared: they come from the band
    /// rather than from the phase.
    #[inline]
    #[allow(
        clippy::cast_precision_loss,
        reason = "harmonic indices are bounded by MAX_HARMONIC and exact in f32"
    )]
    fn lane_limits(&self, steps: f32x8) -> ([usize; 8], [(f32, f32); 8]) {
        let fixed_maximum = self.requested_maximum.min(MAX_HARMONIC);
        let fixed_limit = 0.5 / (fixed_maximum + 1) as f32;
        // Strictly below the limit, not at it: `admits_high_edge` uses the same
        // strict comparison, and the boundary step falls through to the
        // per-lane path with an identical `maximum`.
        let uniform = fixed_maximum > 0
            && (steps.cmp_gt(f32x8::splat(f32::EPSILON)) & steps.cmp_lt(f32x8::splat(fixed_limit)))
                .all();
        if uniform {
            let weights = self.edge_weights(fixed_maximum, true);
            return ([fixed_maximum; 8], [weights; 8]);
        }
        let steps: [f32; 8] = steps.into();
        let mut maximum = [self.low; 8];
        let mut edges = [(0.0, 0.0); 8];
        for lane in 0..8 {
            let step = steps[lane];
            if !step.is_finite() || step <= f32::EPSILON {
                continue;
            }
            let limit = maximum_harmonic(step, self.requested_maximum);
            edges[lane] = self.edge_weights(limit, self.admits_high_edge(limit, step));
            if self.first <= limit {
                maximum[lane] = limit;
            }
        }
        (maximum, edges)
    }
}

/// One saw harmonic of the series `prefixes` accumulates:
/// `-2 / (pi * h) * sin(2 * pi * h * phase)`.
///
/// Evaluated analytically rather than as a difference of two prefix rows so
/// that a fractional edge costs one sine instead of a second set of table
/// gathers, and so the AVX2 range kernels stay untouched.
#[inline]
#[allow(
    clippy::cast_precision_loss,
    reason = "harmonic indices are bounded by MAX_HARMONIC and exact in f32"
)]
fn saw_partial(harmonic: usize, phase: f32) -> f32 {
    if harmonic == 0 || harmonic > MAX_HARMONIC {
        return 0.0;
    }
    let harmonic = harmonic as f32;
    // `aligned_sine_phase(x)` is `-cos(2 * pi * x)`, so shifting by a quarter
    // cycle turns it into `sin(2 * pi * x)`.
    (-2.0 / (std::f32::consts::PI * harmonic)) * aligned_sine_phase(harmonic.mul_add(phase, 0.25))
}

/// One triangle harmonic: `-8 / (pi^2 * h^2) * cos(2 * pi * h * phase)`, and a
/// triangle has no even harmonics.
#[inline]
#[allow(
    clippy::cast_precision_loss,
    reason = "harmonic indices are bounded by MAX_HARMONIC and exact in f32"
)]
fn triangle_partial(harmonic: usize, phase: f32) -> f32 {
    if harmonic == 0 || harmonic > MAX_HARMONIC || harmonic & 1 == 0 {
        return 0.0;
    }
    let harmonic = harmonic as f32;
    (8.0 / (std::f32::consts::PI * std::f32::consts::PI * harmonic * harmonic))
        * aligned_sine_phase(harmonic * phase)
}

/// Eight-lane `saw_partial`. The harmonic is shared across lanes because it
/// comes from the band, not the per-lane phase.
#[inline]
#[allow(
    clippy::cast_precision_loss,
    reason = "harmonic indices are bounded by MAX_HARMONIC and exact in f32"
)]
fn saw_partial8(harmonic: usize, phases: f32x8) -> f32x8 {
    if harmonic == 0 || harmonic > MAX_HARMONIC {
        return f32x8::ZERO;
    }
    let harmonic = harmonic as f32;
    let angle = fract8(f32x8::splat(harmonic) * phases + f32x8::splat(0.25));
    aligned_sine_phase8(angle) * f32x8::splat(-2.0 / (std::f32::consts::PI * harmonic))
}

/// Eight-lane `triangle_partial`.
#[inline]
#[allow(
    clippy::cast_precision_loss,
    reason = "harmonic indices are bounded by MAX_HARMONIC and exact in f32"
)]
fn triangle_partial8(harmonic: usize, phases: f32x8) -> f32x8 {
    if harmonic == 0 || harmonic > MAX_HARMONIC || harmonic & 1 == 0 {
        return f32x8::ZERO;
    }
    let harmonic = harmonic as f32;
    let angle = fract8(f32x8::splat(harmonic) * phases);
    aligned_sine_phase8(angle)
        * f32x8::splat(8.0 / (std::f32::consts::PI * std::f32::consts::PI * harmonic * harmonic))
}

/// `wrap_phase8` only subtracts one cycle, which is enough for advancing a
/// phase but not for `harmonic * phase`, where the product reaches the
/// harmonic count.
#[inline]
fn fract8(phase: f32x8) -> f32x8 {
    phase - phase.floor()
}

pub(super) fn prepare() {
    let _ = prefixes();
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "bounded harmonic and table indices are exactly represented here; offline f64 synthesis is stored as f32 audio"
)]
fn prefixes() -> &'static PrefixBank {
    PREFIXES.get_or_init(|| {
        let mut saw = vec![0.0; ROWS * TABLE_SIZE].into_boxed_slice();
        let mut triangle = vec![0.0; ROWS * TABLE_SIZE].into_boxed_slice();
        for harmonic in 1..=MAX_HARMONIC {
            let previous = (harmonic - 1) * TABLE_SIZE;
            let row = harmonic * TABLE_SIZE;
            saw.copy_within(previous..previous + TABLE_SIZE, row);
            triangle.copy_within(previous..previous + TABLE_SIZE, row);

            let angle = std::f64::consts::TAU * harmonic as f64 / TABLE_SIZE as f64;
            let (step_sine, step_cosine) = angle.sin_cos();
            let mut sine = 0.0_f64;
            let mut cosine = 1.0_f64;
            let saw_gain = -2.0 / (std::f64::consts::PI * harmonic as f64);
            let triangle_gain = if harmonic & 1 == 1 {
                -8.0 / (std::f64::consts::PI * std::f64::consts::PI * (harmonic * harmonic) as f64)
            } else {
                0.0
            };
            for sample in 0..TABLE_SIZE {
                saw[row + sample] += (saw_gain * sine) as f32;
                triangle[row + sample] += (triangle_gain * cosine) as f32;
                (sine, cosine) = (
                    cosine.mul_add(step_sine, sine * step_cosine),
                    sine.mul_add(-step_sine, cosine * step_cosine),
                );
            }
        }
        PrefixBank { saw, triangle }
    })
}

#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "phase step, ratio, and shape are finite and clamped to small positive table ranges"
)]
pub(super) fn sample_shape_ratio(
    shape: f32,
    raw_phase: f32,
    raw_step: f32,
    pulse_width: f32,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    minimum_exclusive: f32,
    maximum_inclusive: f32,
) -> f32 {
    let Some(source) = PreparedRatioSource::new(
        shape,
        pulse_width,
        warp_mode,
        warp_amount,
        (minimum_exclusive, maximum_inclusive),
    ) else {
        return 0.0;
    };
    sample_prepared_ratio(&source, raw_phase, raw_step)
}

#[must_use]
pub(super) fn sample_prepared_ratio(
    source: &PreparedRatioSource,
    raw_phase: f32,
    raw_step: f32,
) -> f32 {
    let (phase, warped_step) =
        warp_phase_scalar(raw_phase, raw_step, source.warp_mode, source.warp_amount);
    let step = warped_step.abs();
    if !step.is_finite() || step <= f32::EPSILON {
        return 0.0;
    }
    let maximum = maximum_harmonic(step, source.requested_maximum);
    let (high_weight, low_weight) =
        source.edge_weights(maximum, source.admits_high_edge(maximum, step));
    let integer_band_empty = source.first > maximum;
    if integer_band_empty && high_weight == 0.0 && low_weight == 0.0 {
        return 0.0;
    }
    let high_harmonic = source.requested_maximum + 1;
    let low_harmonic = source.low + 1;
    let sine = || {
        if source.first == 1 {
            // Harmonic 1 is this segment's morph target, so the lower edge
            // fades it exactly as it fades that harmonic out of the ranges.
            aligned_sine_phase(phase) * (1.0 - source.low_fraction)
        } else {
            0.0
        }
    };
    // Each range is the integer prefix window plus the two analytic edge
    // harmonics, weighted by how far each edge has swept past them.
    let range = |table: &[f32], partial: fn(usize, f32) -> f32, phase: f32| {
        let base = if integer_band_empty {
            0.0
        } else {
            table_range(table, phase, source.low, maximum)
        };
        low_weight.mul_add(
            -partial(low_harmonic, phase),
            high_weight.mul_add(partial(high_harmonic, phase), base),
        )
    };
    let triangle = || range(&source.prefixes.triangle, triangle_partial, phase);
    let saw = || range(&source.prefixes.saw, saw_partial, phase);
    let shifted_saw = || {
        range(
            &source.prefixes.saw,
            saw_partial,
            phase + source.pulse_shift,
        )
    };
    match source.segment {
        0 => {
            let sine = sine();
            if source.blend <= f32::EPSILON {
                sine
            } else {
                source.blend.mul_add(triangle() - sine, sine) * source.gain
            }
        }
        1 => {
            let triangle = triangle();
            if source.blend <= f32::EPSILON {
                triangle
            } else {
                source.blend.mul_add(saw() - triangle, triangle) * source.gain
            }
        }
        2 => {
            let saw = saw();
            if source.blend <= f32::EPSILON {
                saw
            } else {
                source.blend.mul_add(shifted_saw() - saw - saw, saw) * source.gain
            }
        }
        _ => shifted_saw() - saw(),
    }
}

#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "phase steps and harmonic indices are finite and bounded before table access"
)]
pub(super) fn sample_shape_ratio8(
    shape: f32,
    raw_phase: f32x8,
    raw_step: f32x8,
    pulse_width: f32,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    minimum_exclusive: f32,
    maximum_inclusive: f32,
) -> f32x8 {
    let Some(source) = PreparedRatioSource::new(
        shape,
        pulse_width,
        warp_mode,
        warp_amount,
        (minimum_exclusive, maximum_inclusive),
    ) else {
        return f32x8::ZERO;
    };
    sample_prepared_ratio8(&source, raw_phase, raw_step)
}

#[must_use]
fn sample_prepared_ratio8(
    source: &PreparedRatioSource,
    raw_phase: f32x8,
    raw_step: f32x8,
) -> f32x8 {
    let (phases, warped_steps) =
        warp_phase8(raw_phase, raw_step, source.warp_mode, source.warp_amount);
    let phases = wrap_phase8(phases);
    let steps = warped_steps.abs();
    let (maximum, edges) = source.lane_limits(steps);
    let high_weights = f32x8::from(edges.map(|(high, _)| high));
    let low_weights = f32x8::from(edges.map(|(_, low)| low));
    let high_harmonic = source.requested_maximum + 1;
    let low_harmonic = source.low + 1;
    let sine_lanes = || {
        let sine: [f32; 8] = aligned_sine_phase8(phases).into();
        // Harmonic 1 is this segment's morph target, so the lower edge fades it
        // exactly as it fades that harmonic out of the prefix ranges.
        f32x8::from(std::array::from_fn(|lane| {
            if maximum[lane] == 0 { 0.0 } else { sine[lane] }
        })) * f32x8::splat(1.0 - source.low_fraction)
    };
    if source.segment == 0 && source.blend <= f32::EPSILON {
        if source.first != 1 {
            return f32x8::ZERO;
        }
        return sine_lanes();
    }
    let positions = TablePositions8::new(phases);
    let sine = || {
        if source.first == 1 {
            sine_lanes()
        } else {
            f32x8::ZERO
        }
    };
    // Each range is the integer prefix window plus the two analytic edge
    // harmonics, weighted by how far each edge has swept past them.
    let range = |table: &[f32],
                 partial: fn(usize, f32x8) -> f32x8,
                 positions: &TablePositions8,
                 phases: f32x8| {
        let base = table_range8(table, positions, source.low, maximum);
        let base = high_weights.mul_add(partial(high_harmonic, phases), base);
        low_weights.mul_add(-partial(low_harmonic, phases), base)
    };
    let triangle = || {
        range(
            &source.prefixes.triangle,
            triangle_partial8,
            &positions,
            phases,
        )
    };
    let saw = || range(&source.prefixes.saw, saw_partial8, &positions, phases);
    let shifted_saw = || {
        let shifted_phases = wrap_phase8(phases + f32x8::splat(source.pulse_shift));
        let shifted = TablePositions8::new(shifted_phases);
        range(&source.prefixes.saw, saw_partial8, &shifted, shifted_phases)
    };
    let blend = f32x8::splat(source.blend);
    let gain = f32x8::splat(source.gain);
    match source.segment {
        0 => {
            let sine = sine();
            if source.blend <= f32::EPSILON {
                sine
            } else {
                (triangle() - sine).mul_add(blend, sine) * gain
            }
        }
        1 => {
            let triangle = triangle();
            if source.blend <= f32::EPSILON {
                triangle
            } else {
                (saw() - triangle).mul_add(blend, triangle) * gain
            }
        }
        2 => {
            let saw = saw();
            if source.blend <= f32::EPSILON {
                saw
            } else {
                (shifted_saw() - saw - saw).mul_add(blend, saw) * gain
            }
        }
        _ => shifted_saw() - saw(),
    }
}

#[inline(always)]
fn maximum_harmonic(step: f32, requested: usize) -> usize {
    let requested = requested.min(MAX_HARMONIC);
    if !step.is_finite() || step <= f32::EPSILON || requested == 0 {
        return 0;
    }
    if step <= 0.5 / (requested + 1) as f32 {
        return requested;
    }
    let nyquist_ratio = 0.5 / step;
    (f32::from_bits(nyquist_ratio.to_bits() - 1) as usize).min(requested)
}

#[must_use]
fn sample_prepared_ratio4(
    source: &PreparedRatioSource,
    raw_phase: f32x4,
    raw_step: f32x4,
) -> f32x4 {
    let phases: [f32; 4] = raw_phase.into();
    let steps: [f32; 4] = raw_step.into();
    f32x4::from(std::array::from_fn(|lane| {
        sample_prepared_ratio(source, phases[lane], steps[lane])
    }))
}

pub(crate) fn generate_shape8_ratio(
    oscillators: &mut [VaOscillator],
    shape: f32,
    phase_steps: [f32; 8],
    pulse_width: f32,
    warp_mode: PhaseWarpMode,
    warp_amount: f32,
    ratio_band: (f32, f32),
) -> f32x8 {
    debug_assert!(oscillators.len() >= 8);
    let phase = f32x8::from(std::array::from_fn(|lane| oscillators[lane].phase));
    let step = f32x8::from(phase_steps);
    let next = phase + step;
    let advanced = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
    for (oscillator, phase) in oscillators.iter_mut().zip(<[f32; 8]>::from(advanced)) {
        oscillator.phase = phase;
    }
    sample_shape_ratio8(
        shape,
        phase,
        step,
        pulse_width,
        warp_mode,
        warp_amount,
        ratio_band.0,
        ratio_band.1,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the fixed-allocation ratio kernel receives its complete render context"
)]
pub(crate) fn accumulate_shape8_ratio_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x8; SAMPLES],
    left_gain: f32x8,
    right_gain: f32x8,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    source: &PreparedRatioSource,
) {
    debug_assert!(oscillators.len() >= 8);
    let mut phase = f32x8::from(std::array::from_fn(|lane| oscillators[lane].phase));
    for frame in 0..SAMPLES {
        let step = phase_steps[frame];
        let current = phase;
        let next = current + step;
        phase = next.cmp_lt(f32x8::ONE).blend(next, next - f32x8::ONE);
        let sample = sample_prepared_ratio8(source, current, step);
        left[frame] = sample.mul_add(left_gain, left[frame]);
        right[frame] = sample.mul_add(right_gain, right[frame]);
    }
    let phases: [f32; 8] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
        oscillator.phase = phase;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the four-lane tail kernel mirrors the eight-lane ratio render context"
)]
pub(crate) fn accumulate_shape4_ratio_block<const SAMPLES: usize>(
    oscillators: &mut [VaOscillator],
    phase_steps: [f32x4; SAMPLES],
    left_gain: f32x4,
    right_gain: f32x4,
    left: &mut [f32x8; SAMPLES],
    right: &mut [f32x8; SAMPLES],
    source: &PreparedRatioSource,
) {
    debug_assert!(oscillators.len() >= 4);
    let mut phase = f32x4::from(std::array::from_fn(|lane| oscillators[lane].phase));
    for frame in 0..SAMPLES {
        let step = phase_steps[frame];
        let current = phase;
        let next = current + step;
        phase = next.cmp_lt(f32x4::ONE).blend(next, next - f32x4::ONE);
        let sample = sample_prepared_ratio4(source, current, step);
        add4_to8(&mut left[frame], sample * left_gain);
        add4_to8(&mut right[frame], sample * right_gain);
    }
    let phases: [f32; 4] = phase.into();
    for (oscillator, phase) in oscillators.iter_mut().zip(phases) {
        oscillator.phase = phase;
    }
}

#[inline]
fn add4_to8(output: &mut f32x8, contribution: f32x4) {
    let [a, b, c, d]: [f32; 4] = contribution.into();
    *output += f32x8::from([a, b, c, d, 0.0, 0.0, 0.0, 0.0]);
}

#[inline]
fn table_range(table: &[f32], phase: f32, low: usize, high: usize) -> f32 {
    if low == 0 && high <= LINEAR_HARMONIC_LIMIT {
        return eval_linear(table, phase, high);
    }
    let high = eval(table, phase, high);
    if low == 0 {
        high
    } else {
        high - eval(table, phase, low)
    }
}

struct TablePositions8 {
    indices: [usize; 8],
    fractions: f32x8,
}

impl TablePositions8 {
    #[inline(always)]
    fn new(phases: f32x8) -> Self {
        let phases: [f32; 8] = phases.into();
        let positions = phases.map(|phase| phase * TABLE_SIZE as f32);
        let indices = positions.map(|position| (position as usize).min(TABLE_SIZE - 1));
        let fractions = f32x8::from(std::array::from_fn(|lane| {
            positions[lane] - indices[lane] as f32
        }));
        Self { indices, fractions }
    }
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "wrapped positions and harmonic rows are bounded by the immutable prefix table"
)]
fn table_range8(table: &[f32], positions: &TablePositions8, low: usize, high: [usize; 8]) -> f32x8 {
    let linear = low == 0
        && high
            .iter()
            .all(|&harmonic| harmonic <= LINEAR_HARMONIC_LIMIT);
    #[cfg(target_arch = "x86_64")]
    if crate::performance::spline_backend() == crate::performance::SplineBackend::Avx2Fma {
        if linear && high[1..].iter().all(|&harmonic| harmonic == high[0]) {
            // SAFETY: the runtime backend is published only after AVX2 and FMA are
            // detected; harmonic rows and wrapped sample indices are table-bounded.
            return unsafe {
                eval_linear8_avx2(table, high[0], positions.indices, positions.fractions)
            };
        }
        // SAFETY: the runtime backend is published only after AVX2 and FMA are
        // detected; harmonic rows and wrapped sample indices are table-bounded.
        return unsafe {
            table_range8_avx2(
                table,
                high,
                low,
                positions.indices,
                positions.fractions,
                linear,
            )
        };
    }
    let tap = |offset: usize| {
        f32x8::from(std::array::from_fn(|lane| {
            let index = (positions.indices[lane] + offset) & TABLE_MASK;
            let high = table[high[lane].min(MAX_HARMONIC) * TABLE_SIZE + index];
            if low == 0 {
                high
            } else {
                high - table[low.min(MAX_HARMONIC) * TABLE_SIZE + index]
            }
        }))
    };
    if linear {
        let p1 = tap(0);
        return (tap(1) - p1).mul_add(positions.fractions, p1);
    }
    let p0 = tap(TABLE_SIZE - 1);
    let p1 = tap(0);
    let p2 = tap(1);
    let p3 = tap(2);
    let half = f32x8::splat(0.5);
    let a = (-p0 + p1 * f32x8::splat(3.0) - p2 * f32x8::splat(3.0) + p3) * half;
    let b = (p0 * f32x8::splat(2.0) - p1 * f32x8::splat(5.0) + p2 * f32x8::splat(4.0) - p3) * half;
    let c = (p2 - p0) * half;
    a.mul_add(positions.fractions, b)
        .mul_add(positions.fractions, c)
        .mul_add(positions.fractions, p1)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(
    unsafe_op_in_unsafe_fn,
    reason = "the runtime-guarded uniform-row lookup uses table-bounded AVX2 gathers"
)]
unsafe fn eval_linear8_avx2(
    table: &[f32],
    harmonic: usize,
    indices: [usize; 8],
    fractions: f32x8,
) -> f32x8 {
    use core::arch::x86_64::*;

    let row = table.as_ptr().add(harmonic.min(MAX_HARMONIC) * TABLE_SIZE);
    let indices = indices.map(|index| index as i32);
    let next = indices.map(|index| (index + 1) & TABLE_MASK as i32);
    let indices = _mm256_loadu_si256(indices.as_ptr().cast());
    let next = _mm256_loadu_si256(next.as_ptr().cast());
    let p1 = _mm256_i32gather_ps::<4>(row, indices);
    let p2 = _mm256_i32gather_ps::<4>(row, next);
    let fractions: [f32; 8] = fractions.into();
    let value = _mm256_fmadd_ps(
        _mm256_sub_ps(p2, p1),
        _mm256_loadu_ps(fractions.as_ptr()),
        p1,
    );
    let mut output = [0.0; 8];
    _mm256_storeu_ps(output.as_mut_ptr(), value);
    f32x8::from(output)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(
    unsafe_op_in_unsafe_fn,
    reason = "the runtime-guarded ratio lookup keeps its AVX2 gather kernel in one unit"
)]
unsafe fn table_range8_avx2(
    table: &[f32],
    high: [usize; 8],
    low: usize,
    indices: [usize; 8],
    fractions: f32x8,
    linear: bool,
) -> f32x8 {
    use core::arch::x86_64::*;

    let indices = indices.map(|index| index as i32);
    let indices = _mm256_loadu_si256(indices.as_ptr().cast());
    let high_rows = high.map(|harmonic| (harmonic.min(MAX_HARMONIC) * TABLE_SIZE) as i32);
    let high_rows = _mm256_loadu_si256(high_rows.as_ptr().cast());
    let low_row = _mm256_set1_epi32((low.min(MAX_HARMONIC) * TABLE_SIZE) as i32);
    let mask = _mm256_set1_epi32(TABLE_MASK as i32);
    let range = |offset| {
        let tap = _mm256_and_si256(
            _mm256_add_epi32(indices, _mm256_set1_epi32(offset as i32)),
            mask,
        );
        let high = _mm256_i32gather_ps::<4>(table.as_ptr(), _mm256_add_epi32(high_rows, tap));
        if low == 0 {
            high
        } else {
            _mm256_sub_ps(
                high,
                _mm256_i32gather_ps::<4>(table.as_ptr(), _mm256_add_epi32(low_row, tap)),
            )
        }
    };
    let fractions: [f32; 8] = fractions.into();
    let fractions = _mm256_loadu_ps(fractions.as_ptr());
    if linear {
        let p1 = range(0);
        let value = _mm256_fmadd_ps(_mm256_sub_ps(range(1), p1), fractions, p1);
        let mut output = [0.0; 8];
        _mm256_storeu_ps(output.as_mut_ptr(), value);
        return f32x8::from(output);
    }
    let p0 = range(TABLE_SIZE - 1);
    let p1 = range(0);
    let p2 = range(1);
    let p3 = range(2);
    let half = _mm256_set1_ps(0.5);
    let a = _mm256_mul_ps(
        _mm256_sub_ps(
            _mm256_sub_ps(p3, p0),
            _mm256_mul_ps(_mm256_sub_ps(p2, p1), _mm256_set1_ps(3.0)),
        ),
        half,
    );
    let b = _mm256_mul_ps(
        _mm256_sub_ps(
            _mm256_add_ps(
                _mm256_mul_ps(p0, _mm256_set1_ps(2.0)),
                _mm256_mul_ps(p2, _mm256_set1_ps(4.0)),
            ),
            _mm256_add_ps(_mm256_mul_ps(p1, _mm256_set1_ps(5.0)), p3),
        ),
        half,
    );
    let c = _mm256_mul_ps(_mm256_sub_ps(p2, p0), half);
    let value = _mm256_fmadd_ps(
        _mm256_fmadd_ps(_mm256_fmadd_ps(a, fractions, b), fractions, c),
        fractions,
        p1,
    );
    let mut output = [0.0; 8];
    _mm256_storeu_ps(output.as_mut_ptr(), value);
    f32x8::from(output)
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "wrapped positions are finite and bounded to the 2048-sample cycle"
)]
fn eval(table: &[f32], phase: f32, harmonic: usize) -> f32 {
    let position = (phase - phase.floor()) * TABLE_SIZE as f32;
    let index = (position as usize).min(TABLE_SIZE - 1);
    let fraction = position - index as f32;
    let row = harmonic.min(MAX_HARMONIC) * TABLE_SIZE;
    let p0 = table[row + ((index + TABLE_SIZE - 1) & TABLE_MASK)];
    let p1 = table[row + index];
    let p2 = table[row + ((index + 1) & TABLE_MASK)];
    let p3 = table[row + ((index + 2) & TABLE_MASK)];
    let a = (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * 0.5;
    let b = (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * 0.5;
    let c = (p2 - p0) * 0.5;
    a.mul_add(fraction, b)
        .mul_add(fraction, c)
        .mul_add(fraction, p1)
}

#[inline]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "wrapped positions are finite and bounded to the 2048-sample cycle"
)]
fn eval_linear(table: &[f32], phase: f32, harmonic: usize) -> f32 {
    let position = (phase - phase.floor()) * TABLE_SIZE as f32;
    let index = (position as usize).min(TABLE_SIZE - 1);
    let fraction = position - index as f32;
    let row = harmonic.min(MAX_HARMONIC) * TABLE_SIZE;
    let p1 = table[row + index];
    (table[row + ((index + 1) & TABLE_MASK)] - p1).mul_add(fraction, p1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPE_SAW: f32 = 2.0;
    const STEP: f32 = 1.0 / 512.0;

    fn sample(shape: f32, phase: f32, low: f32, high: f32) -> f32 {
        prepare();
        sample_shape_ratio(shape, phase, STEP, 0.5, PhaseWarpMode::None, 0.0, low, high)
    }

    fn sample8(shape: f32, phase: f32, low: f32, high: f32) -> f32 {
        prepare();
        let lanes: [f32; 8] = sample_shape_ratio8(
            shape,
            f32x8::splat(phase),
            f32x8::splat(STEP),
            0.5,
            PhaseWarpMode::None,
            0.0,
            low,
            high,
        )
        .into();
        lanes[0]
    }

    /// The band edges are continuous controls. Before the fractional edge
    /// weighting, sweeping one past a harmonic switched that harmonic on in a
    /// single sample, so the output jumped by the harmonic's full amplitude --
    /// a click per partial under cutoff modulation.
    #[test]
    fn sweeping_the_upper_edge_never_steps_by_a_whole_harmonic() {
        for &shape in &[1.0_f32, SHAPE_SAW, 3.0] {
            for &phase in &[0.0_f32, 0.13, 0.37, 0.61, 0.88] {
                let mut previous = sample(shape, phase, 0.0, 2.0);
                let mut largest_jump = 0.0_f32;
                // Two harmonics of travel at 1/256 of a harmonic per step.
                for tick in 1..=512_u16 {
                    let high = 2.0 + f32::from(tick) / 256.0;
                    let value = sample(shape, phase, 0.0, high);
                    largest_jump = largest_jump.max((value - previous).abs());
                    previous = value;
                }
                // Harmonic 3 of a saw peaks at 2 / (3 * pi) ~= 0.212; one
                // 1/256 sweep step may only move a small fraction of that.
                assert!(
                    largest_jump < 0.01,
                    "shape {shape} phase {phase}: upper edge stepped by {largest_jump}"
                );
            }
        }
    }

    #[test]
    fn sweeping_the_lower_edge_never_steps_by_a_whole_harmonic() {
        for &shape in &[1.0_f32, SHAPE_SAW, 3.0] {
            for &phase in &[0.0_f32, 0.13, 0.37, 0.61, 0.88] {
                let mut previous = sample(shape, phase, 0.0, 16.0);
                let mut largest_jump = 0.0_f32;
                for tick in 1..=512_u16 {
                    let low = f32::from(tick) / 256.0;
                    let value = sample(shape, phase, low, 16.0);
                    largest_jump = largest_jump.max((value - previous).abs());
                    previous = value;
                }
                assert!(
                    largest_jump < 0.01,
                    "shape {shape} phase {phase}: lower edge stepped by {largest_jump}"
                );
            }
        }
    }

    /// The edge term is synthesized from the same series the prefix tables sum,
    /// so a fully swept edge has to land on the next prefix row. Any drift here
    /// means the analytic partial and the table disagree.
    #[test]
    fn a_fully_swept_edge_matches_the_next_prefix_row() {
        for &shape in &[1.0_f32, SHAPE_SAW, 3.0] {
            for harmonic in 1..12_u16 {
                for &phase in &[0.0_f32, 0.13, 0.37, 0.61, 0.88] {
                    let edge = f32::from(harmonic);
                    let swept = sample(shape, phase, 0.0, edge - 1.0e-4);
                    let integral = sample(shape, phase, 0.0, edge);
                    assert!(
                        (swept - integral).abs() < 2.0e-3,
                        "shape {shape} harmonic {harmonic} phase {phase}: \
                         swept {swept} vs prefix row {integral}"
                    );
                }
            }
        }
    }

    /// Integer bands must be untouched: they are what every existing ratio
    /// expectation is written against.
    #[test]
    fn integer_bands_carry_no_edge_term() {
        for &shape in &[0.0_f32, 1.0, SHAPE_SAW, 3.0] {
            for &phase in &[0.0_f32, 0.13, 0.37, 0.61, 0.88] {
                for &(low, high) in &[(0.0_f32, 1.0_f32), (0.0, 8.0), (3.0, 9.0), (0.0, 64.0)] {
                    let scalar = sample(shape, phase, low, high);
                    let wide = sample8(shape, phase, low, high);
                    assert!(
                        (scalar - wide).abs() < 1.0e-6,
                        "shape {shape} phase {phase} band ({low}, {high}]: \
                         scalar {scalar} vs eight-lane {wide}"
                    );
                }
            }
        }
    }

    /// The eight-lane kernel weights its edges per lane; on a uniform phase and
    /// step it must agree with the scalar kernel on fractional bands too.
    #[test]
    fn fractional_bands_agree_across_widths() {
        for &shape in &[0.0_f32, 1.0, SHAPE_SAW, 3.0] {
            for &phase in &[0.0_f32, 0.13, 0.37, 0.61, 0.88] {
                for &(low, high) in &[
                    (0.0_f32, 2.4_f32),
                    (0.35, 7.9),
                    (2.75, 3.25),
                    (4.5, 4.5),
                    (0.6, 40.2),
                ] {
                    let scalar = sample(shape, phase, low, high);
                    let wide = sample8(shape, phase, low, high);
                    assert!(
                        (scalar - wide).abs() < 1.0e-5,
                        "shape {shape} phase {phase} band ({low}, {high}]: \
                         scalar {scalar} vs eight-lane {wide}"
                    );
                }
            }
        }
    }

    /// A band that straddles one harmonic must fade that harmonic in from
    /// silence as the band opens, rather than admitting it whole.
    #[test]
    fn a_band_around_one_harmonic_fades_it_in() {
        let mut previous = 0.0_f32;
        for tick in 0..=64_u16 {
            let half_width = f32::from(tick) / 128.0;
            let value = sample(SHAPE_SAW, 0.37, 3.0 - half_width, 3.0 + half_width).abs();
            assert!(
                value + 1.0e-6 >= previous,
                "half width {half_width}: {value} is below the narrower band's {previous}"
            );
            previous = value;
        }
        assert!(
            sample(SHAPE_SAW, 0.37, 3.0, 3.0).abs() < 1.0e-6,
            "a zero-width band must be silent"
        );
    }

    /// A band narrower than one harmonic falls entirely between two of them.
    /// Both ramps then land on the same partial, so its weight is their product
    /// and it can never exceed what either edge alone would allow.
    #[test]
    fn a_sub_harmonic_band_is_bounded_by_both_of_its_edges() {
        for &phase in &[0.0_f32, 0.13, 0.37, 0.61, 0.88] {
            let whole = saw_partial(4, phase).abs();
            // Band (3.2, 3.7]: harmonic 4 is 0.7 of the way in from above and
            // 0.8 of the way in from below.
            let value = sample(SHAPE_SAW, phase, 3.2, 3.7).abs();
            let bound = 0.7 * 0.8 * whole;
            assert!(
                value <= bound + 1.0e-6,
                "phase {phase}: {value} exceeds the product bound {bound}"
            );
            assert!(
                sample(SHAPE_SAW, phase, 3.05, 3.1).abs() <= value + 1.0e-6,
                "phase {phase}: a narrower sub-harmonic band must not be louder"
            );
        }
    }
}
