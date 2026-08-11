//! Editable periodic oscillator curve compiled to fixed realtime coefficients.

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
pub(crate) const MIN_WAVE_KNOTS: usize = 3;
const MIN_SPACING: f32 = 0.015;
const DRAW_FIT_SAMPLES: usize = 256;
const DRAW_FIT_TOLERANCE: f32 = 0.0125;

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
    /// Shape of the segment leaving this knot. Zero is the legacy spline.
    pub curve: f32,
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
                },
                WaveKnot {
                    phase: 0.25,
                    value: 1.0,
                    curve: 0.0,
                },
                WaveKnot {
                    phase: 0.5,
                    value: 0.0,
                    curve: 0.0,
                },
                WaveKnot {
                    phase: 0.75,
                    value: -1.0,
                    curve: 0.0,
                },
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveCurveRt {
    coefficients: [f32; RT_VALUES],
}

impl Default for WaveCurveRt {
    fn default() -> Self {
        WaveCurveData::default().compile_rt()
    }
}

impl WaveCurveData {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            knots: sanitize_knots(&self.knots),
        }
    }

    pub fn compile_rt(&self) -> WaveCurveRt {
        let source = SourceCurve::compile(&sanitize_knots(&self.knots));
        let controls = fit_periodic_bspline(&source);
        WaveCurveRt::from_controls(controls)
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
}

impl SourceCurve {
    fn compile(knots: &[WaveKnot]) -> Self {
        let count = knots.len();
        let mut widths = [0.0; MAX_WAVE_KNOTS];
        let mut secants = [0.0; MAX_WAVE_KNOTS];
        for index in 0..count {
            let next = (index + 1) % count;
            let x0 = knots[index].phase;
            let x1 = if next == 0 { 1.0 } else { knots[next].phase };
            widths[index] = (x1 - x0).max(MIN_SPACING);
            secants[index] = (knots[next].value - knots[index].value) / widths[index];
        }
        let mut tangent = [0.0; MAX_WAVE_KNOTS];
        for index in 0..count {
            let previous = (index + count - 1) % count;
            let before = secants[previous];
            let after = secants[index];
            if before * after > 0.0 {
                let before_weight = 2.0 * widths[index] + widths[previous];
                let after_weight = widths[index] + 2.0 * widths[previous];
                tangent[index] = (before_weight + after_weight)
                    / (before_weight / before + after_weight / after);
            }
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
        };
        for index in 0..count {
            let next = (index + 1) % count;
            let x0 = knots[index].phase;
            let x1 = if next == 0 { 1.0 } else { knots[next].phase };
            let width = widths[index];
            let y0 = knots[index].value;
            let y1 = knots[next].value;
            let m0 = tangent[index] * width;
            let m1 = tangent[next] * width;
            result.x0[index] = x0;
            result.x1[index] = x1;
            result.inverse_width[index] = width.recip();
            result.a[index] = 2.0 * y0 - 2.0 * y1 + m0 + m1;
            result.b[index] = -3.0 * y0 + 3.0 * y1 - 2.0 * m0 - m1;
            result.c[index] = m0;
            result.d[index] = y0;
            result.curve[index] = knots[index].curve;
        }
        result
    }

    fn eval(&self, phase: f64) -> f64 {
        for index in 0..self.count {
            if phase < f64::from(self.x1[index]) {
                let t = (phase - f64::from(self.x0[index])) * f64::from(self.inverse_width[index]);
                let curve = f64::from(self.curve[index]);
                let t = t + curve * t * (1.0 - t);
                return f64::from(self.a[index])
                    .mul_add(t, f64::from(self.b[index]))
                    .mul_add(t, f64::from(self.c[index]))
                    .mul_add(t, f64::from(self.d[index]));
            }
        }
        f64::from(self.d[0])
    }
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

    fn from_controls(controls: [f32; MAX_WAVE_KNOTS]) -> Self {
        let mut coefficients = [0.0; RT_VALUES];
        for index in 0..RT_SEGMENTS {
            let p0 = controls[(index + RT_SEGMENTS - 1) % RT_SEGMENTS];
            let p1 = controls[index];
            let p2 = controls[(index + 1) % RT_SEGMENTS];
            let p3 = controls[(index + 2) % RT_SEGMENTS];
            coefficients[coefficient_index(index, 0)] = (-p0 + 3.0 * p1 - 3.0 * p2 + p3) / 6.0;
            coefficients[coefficient_index(index, 1)] = (3.0 * p0 - 6.0 * p1 + 3.0 * p2) / 6.0;
            coefficients[coefficient_index(index, 2)] = (-3.0 * p0 + 3.0 * p2) / 6.0;
            coefficients[coefficient_index(index, 3)] = (p0 + 4.0 * p1 + p2) / 6.0;
        }
        Self { coefficients }
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

    pub(crate) fn max_difference(self, other: Self) -> f32 {
        self.coefficients
            .into_iter()
            .zip(other.coefficients)
            .fold(0.0_f32, |difference, (left, right)| {
                difference.max((left - right).abs())
            })
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
        self.eval_raw(phase)
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
            a.mul_add(t, b).mul_add(t, c).mul_add(t, d)
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
                ((a * t + b) * t + c) * t + d
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

fn fit_periodic_bspline(source: &SourceCurve) -> [f32; RT_SEGMENTS] {
    const GAUSS_LEGENDRE_4: [(f64, f64); 4] = [
        (-0.861_136_311_594_052_6, 0.347_854_845_137_453_8),
        (-0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
        (0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
        (0.861_136_311_594_052_6, 0.347_854_845_137_453_8),
    ];

    let mut boundaries = (0..=RT_SEGMENTS)
        .map(|index| index as f64 / RT_SEGMENTS as f64)
        .chain(source.x1[..source.count].iter().copied().map(f64::from))
        .collect::<Vec<_>>();
    boundaries.sort_by(f64::total_cmp);
    boundaries.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON);

    let mut matrix = [[0.0_f64; RT_SEGMENTS + 1]; RT_SEGMENTS];
    for interval in boundaries.windows(2) {
        let center = (interval[0] + interval[1]) * 0.5;
        let half_width = (interval[1] - interval[0]) * 0.5;
        for (node, weight) in GAUSS_LEGENDRE_4 {
            let phase = node.mul_add(half_width, center);
            let position = phase * RT_SEGMENTS as f64;
            let segment = (position as usize).min(RT_SEGMENTS - 1);
            let t = position - segment as f64;
            let one_minus_t = 1.0 - t;
            let basis = [
                one_minus_t.powi(3) / 6.0,
                (3.0 * t.powi(3) - 6.0 * t * t + 4.0) / 6.0,
                (-3.0 * t.powi(3) + 3.0 * t * t + 3.0 * t + 1.0) / 6.0,
                t.powi(3) / 6.0,
            ];
            let controls = [
                (segment + RT_SEGMENTS - 1) % RT_SEGMENTS,
                segment,
                (segment + 1) % RT_SEGMENTS,
                (segment + 2) % RT_SEGMENTS,
            ];
            let weighted_width = weight * half_width;
            let target = source.eval(phase);
            for row in 0..4 {
                matrix[controls[row]][RT_SEGMENTS] += weighted_width * basis[row] * target;
                for column in 0..4 {
                    matrix[controls[row]][controls[column]] +=
                        weighted_width * basis[row] * basis[column];
                }
            }
        }
    }
    solve_system(matrix).map(|control| control as f32)
}

fn solve_system(mut matrix: [[f64; RT_SEGMENTS + 1]; RT_SEGMENTS]) -> [f64; RT_SEGMENTS] {
    for column in 0..RT_SEGMENTS {
        let pivot = (column..RT_SEGMENTS)
            .max_by(|left, right| {
                matrix[*left][column]
                    .abs()
                    .total_cmp(&matrix[*right][column].abs())
            })
            .unwrap_or(column);
        matrix.swap(column, pivot);
        let inverse = matrix[column][column].recip();
        for value in &mut matrix[column][column..] {
            *value *= inverse;
        }
        for row in 0..RT_SEGMENTS {
            if row != column {
                let scale = matrix[row][column];
                for entry in column..=RT_SEGMENTS {
                    matrix[row][entry] -= scale * matrix[column][entry];
                }
            }
        }
    }
    std::array::from_fn(|index| matrix[index][RT_SEGMENTS])
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
        let data = WaveCurveData::default();
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
    if selected_count < 4 {
        selected[DRAW_FIT_SAMPLES / 3] = true;
        selected[DRAW_FIT_SAMPLES * 2 / 3] = true;
    }
    let knots = (0..DRAW_FIT_SAMPLES)
        .filter(|&index| selected[index])
        .map(|index| WaveKnot {
            phase: index as f32 / DRAW_FIT_SAMPLES as f32,
            value: samples[index],
            curve: 0.0,
        })
        .collect::<Vec<_>>();
    let mut result = WaveCurveData {
        knots: sanitize_knots(&knots),
    };
    while result.knots.len() < MAX_WAVE_KNOTS {
        let compiled = result.compile_rt();
        let candidate = samples
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                let phase = *index as f32 / DRAW_FIT_SAMPLES as f32;
                result
                    .knots
                    .iter()
                    .all(|knot| (knot.phase - phase).abs() >= MIN_SPACING)
            })
            .map(|(index, sample)| {
                let phase = index as f32 / DRAW_FIT_SAMPLES as f32;
                (phase, *sample, (compiled.eval(phase) - sample).abs())
            })
            .max_by(|left, right| left.2.total_cmp(&right.2));
        let Some((phase, value, error)) = candidate else {
            break;
        };
        if error <= DRAW_FIT_TOLERANCE {
            break;
        }
        if !insert_knot(&mut result, phase, value) {
            break;
        }
    }
    result
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

pub fn set_segment_curve(data: &mut WaveCurveData, index: usize, curve: f32) -> bool {
    let Some(knot) = data.knots.get_mut(index) else {
        return false;
    };
    knot.curve = curve.clamp(-1.0, 1.0);
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
                knot.curve.clamp(-1.0, 1.0)
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
