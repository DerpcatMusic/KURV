//! Immutable harmonic-prefix tables for ratio-domain VA filtering.

use std::sync::OnceLock;

use super::antialias::{aligned_sine_phase, aligned_sine_phase8, wrap_phase8};
use super::warp::warp_phase8;
use super::{PhaseWarpMode, VaOscillator, shape_morph_gain, warp_phase_scalar};
use truce_simd::simd::{f32x4, f32x8};
use wide::{CmpGt, CmpLe, CmpLt};

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
    requested_maximum: usize,
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
        let first = (ratio_band.0.clamp(MIN_RATIO, MAX_RATIO).floor() as usize).saturating_add(1);
        Some(Self {
            prefixes: PREFIXES.get()?,
            segment,
            blend: shape - segment as f32,
            gain: shape_morph_gain(shape),
            pulse_shift: 1.0 - pulse_width.clamp(0.03, 0.97),
            warp_mode,
            warp_amount,
            first,
            low: first.saturating_sub(1),
            requested_maximum: ratio_band.1.clamp(MIN_RATIO, MAX_RATIO).floor() as usize,
        })
    }
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
    if source.first > maximum {
        return 0.0;
    }
    let sine = || {
        if source.first == 1 {
            aligned_sine_phase(phase)
        } else {
            0.0
        }
    };
    let triangle = || table_range(&source.prefixes.triangle, phase, source.low, maximum);
    let saw = || table_range(&source.prefixes.saw, phase, source.low, maximum);
    let shifted_saw = || {
        table_range(
            &source.prefixes.saw,
            phase + source.pulse_shift,
            source.low,
            maximum,
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
    let fixed_maximum = source.requested_maximum.min(MAX_HARMONIC);
    let fixed_limit = 0.5 / (fixed_maximum + 1) as f32;
    let maximum = if fixed_maximum > 0
        && (steps.cmp_gt(f32x8::splat(f32::EPSILON)) & steps.cmp_le(f32x8::splat(fixed_limit)))
            .all()
    {
        [fixed_maximum; 8]
    } else {
        let steps: [f32; 8] = steps.into();
        std::array::from_fn(|lane| {
            let step = steps[lane];
            if !step.is_finite() || step <= f32::EPSILON {
                return source.low;
            }
            let maximum = maximum_harmonic(step, source.requested_maximum);
            if source.first > maximum {
                source.low
            } else {
                maximum
            }
        })
    };
    if source.segment == 0 && source.blend <= f32::EPSILON {
        if source.first != 1 {
            return f32x8::ZERO;
        }
        let sine: [f32; 8] = aligned_sine_phase8(phases).into();
        return f32x8::from(std::array::from_fn(|lane| {
            if maximum[lane] == 0 { 0.0 } else { sine[lane] }
        }));
    }
    let positions = TablePositions8::new(phases);
    let sine = || {
        if source.first == 1 {
            let sine: [f32; 8] = aligned_sine_phase8(phases).into();
            f32x8::from(std::array::from_fn(|lane| {
                if maximum[lane] == 0 { 0.0 } else { sine[lane] }
            }))
        } else {
            f32x8::ZERO
        }
    };
    let triangle = || table_range8(&source.prefixes.triangle, &positions, source.low, maximum);
    let saw = || table_range8(&source.prefixes.saw, &positions, source.low, maximum);
    let shifted_saw = || {
        let shifted = TablePositions8::new(wrap_phase8(phases + f32x8::splat(source.pulse_shift)));
        table_range8(&source.prefixes.saw, &shifted, source.low, maximum)
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
        // SAFETY: the runtime backend is published only after AVX2 and FMA are
        // detected; harmonic rows and wrapped sample indices are table-bounded.
        if linear && high[1..].iter().all(|&harmonic| harmonic == high[0]) {
            return unsafe {
                eval_linear8_avx2(table, high[0], positions.indices, positions.fractions)
            };
        }
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
