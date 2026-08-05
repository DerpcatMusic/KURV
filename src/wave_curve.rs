//! Editable periodic oscillator curve compiled to fixed realtime coefficients.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};
use truce_simd::simd::{f32x4, f32x8};
use wide::CmpGt;

pub const MAX_WAVE_KNOTS: usize = 8;
const COEFFICIENTS_PER_SEGMENT: usize = 4;
const RT_VALUES: usize = MAX_WAVE_KNOTS * COEFFICIENTS_PER_SEGMENT;
const MIN_WAVE_KNOTS: usize = 3;
const MIN_SPACING: f32 = 0.015;

#[derive(Clone, Copy, Debug, Default, PartialEq, State)]
pub struct WaveKnot {
    pub phase: f32,
    pub value: f32,
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
                },
                WaveKnot {
                    phase: 0.25,
                    value: 1.0,
                },
                WaveKnot {
                    phase: 0.5,
                    value: 0.0,
                },
                WaveKnot {
                    phase: 0.75,
                    value: -1.0,
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
    pub fn compile_rt(&self) -> WaveCurveRt {
        let source = SourceCurve::compile(&sanitize_knots(&self.knots));
        let mut targets =
            std::array::from_fn(|index| source.eval(index as f32 / MAX_WAVE_KNOTS as f32));
        let dc = targets.iter().sum::<f32>() / MAX_WAVE_KNOTS as f32;
        for value in &mut targets {
            *value -= dc;
        }
        let controls = solve_periodic_bspline(targets);
        let mut rt = WaveCurveRt::from_controls(controls);
        let mut peak = 0.0_f32;
        for index in 0..256 {
            peak = peak.max(rt.eval_raw(index as f32 / 256.0).abs());
        }
        let gain = peak.max(1.0).recip();
        for value in &mut rt.coefficients {
            *value *= gain;
        }
        rt
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
        }
        result
    }

    fn eval(&self, phase: f32) -> f32 {
        for index in 0..self.count {
            if phase < self.x1[index] {
                let t = (phase - self.x0[index]) * self.inverse_width[index];
                return self.a[index]
                    .mul_add(t, self.b[index])
                    .mul_add(t, self.c[index])
                    .mul_add(t, self.d[index]);
            }
        }
        self.d[0]
    }
}

impl WaveCurveRt {
    pub const fn zero() -> Self {
        Self {
            coefficients: [0.0; RT_VALUES],
        }
    }

    fn from_controls(controls: [f32; MAX_WAVE_KNOTS]) -> Self {
        let mut coefficients = [0.0; RT_VALUES];
        for index in 0..MAX_WAVE_KNOTS {
            let p0 = controls[(index + MAX_WAVE_KNOTS - 1) % MAX_WAVE_KNOTS];
            let p1 = controls[index];
            let p2 = controls[(index + 1) % MAX_WAVE_KNOTS];
            let p3 = controls[(index + 2) % MAX_WAVE_KNOTS];
            let offset = index * COEFFICIENTS_PER_SEGMENT;
            coefficients[offset] = (-p0 + 3.0 * p1 - 3.0 * p2 + p3) / 6.0;
            coefficients[offset + 1] = (3.0 * p0 - 6.0 * p1 + 3.0 * p2) / 6.0;
            coefficients[offset + 2] = (-3.0 * p0 + 3.0 * p2) / 6.0;
            coefficients[offset + 3] = (p0 + 4.0 * p1 + p2) / 6.0;
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

    #[inline]
    fn eval_raw(&self, phase: f32) -> f32 {
        let position = phase * MAX_WAVE_KNOTS as f32;
        let index = (position as usize).min(MAX_WAVE_KNOTS - 1);
        let t = position - index as f32;
        let offset = index * COEFFICIENTS_PER_SEGMENT;
        self.coefficients[offset]
            .mul_add(t, self.coefficients[offset + 1])
            .mul_add(t, self.coefficients[offset + 2])
            .mul_add(t, self.coefficients[offset + 3])
    }

    #[inline]
    pub fn eval(&self, phase: f32) -> f32 {
        self.eval_raw(phase)
    }

    #[inline]
    pub fn eval4(&self, phase: f32x4) -> f32x4 {
        let (index, [a, b, c, d]) = self.select4(phase);
        let t = phase.mul_add(f32x4::splat(MAX_WAVE_KNOTS as f32), -index);
        a.mul_add(t, b).mul_add(t, c).mul_add(t, d)
    }

    #[inline]
    pub fn eval8(&self, phase: f32x8) -> f32x8 {
        let (index, [a, b, c, d]) = self.select8(phase);
        let t = phase.mul_add(f32x8::splat(MAX_WAVE_KNOTS as f32), -index);
        a.mul_add(t, b).mul_add(t, c).mul_add(t, d)
    }

    #[inline]
    fn select4(&self, phase: f32x4) -> (f32x4, [f32x4; COEFFICIENTS_PER_SEGMENT]) {
        let mut index = f32x4::ZERO;
        let mut selected =
            std::array::from_fn(|coefficient| f32x4::splat(self.coefficients[coefficient]));
        for segment in 1..MAX_WAVE_KNOTS {
            let mask = phase.cmp_gt(f32x4::splat(segment as f32 / MAX_WAVE_KNOTS as f32));
            index = mask.blend(f32x4::splat(segment as f32), index);
            let offset = segment * COEFFICIENTS_PER_SEGMENT;
            for coefficient in 0..COEFFICIENTS_PER_SEGMENT {
                selected[coefficient] = mask.blend(
                    f32x4::splat(self.coefficients[offset + coefficient]),
                    selected[coefficient],
                );
            }
        }
        (index, selected)
    }

    #[inline]
    fn select8(&self, phase: f32x8) -> (f32x8, [f32x8; COEFFICIENTS_PER_SEGMENT]) {
        let mut index = f32x8::ZERO;
        let mut selected =
            std::array::from_fn(|coefficient| f32x8::splat(self.coefficients[coefficient]));
        for segment in 1..MAX_WAVE_KNOTS {
            let mask = phase.cmp_gt(f32x8::splat(segment as f32 / MAX_WAVE_KNOTS as f32));
            index = mask.blend(f32x8::splat(segment as f32), index);
            let offset = segment * COEFFICIENTS_PER_SEGMENT;
            for coefficient in 0..COEFFICIENTS_PER_SEGMENT {
                selected[coefficient] = mask.blend(
                    f32x8::splat(self.coefficients[offset + coefficient]),
                    selected[coefficient],
                );
            }
        }
        (index, selected)
    }
}

fn solve_periodic_bspline(samples: [f32; MAX_WAVE_KNOTS]) -> [f32; MAX_WAVE_KNOTS] {
    let mut matrix = [[0.0_f32; MAX_WAVE_KNOTS + 1]; MAX_WAVE_KNOTS];
    for row in 0..MAX_WAVE_KNOTS {
        matrix[row][row] = 4.0;
        matrix[row][(row + MAX_WAVE_KNOTS - 1) % MAX_WAVE_KNOTS] = 1.0;
        matrix[row][(row + 1) % MAX_WAVE_KNOTS] = 1.0;
        matrix[row][MAX_WAVE_KNOTS] = samples[row] * 6.0;
    }
    for column in 0..MAX_WAVE_KNOTS {
        let pivot = (column..MAX_WAVE_KNOTS)
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
        for row in 0..MAX_WAVE_KNOTS {
            if row != column {
                let scale = matrix[row][column];
                for entry in column..=MAX_WAVE_KNOTS {
                    matrix[row][entry] -= scale * matrix[column][entry];
                }
            }
        }
    }
    std::array::from_fn(|index| matrix[index][MAX_WAVE_KNOTS])
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

    pub fn replace(&self, data: WaveCurveData) {
        let data = WaveCurveData {
            knots: sanitize_knots(&data.knots),
        };
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

fn sanitize_knots(knots: &[WaveKnot]) -> Vec<WaveKnot> {
    let mut result = knots
        .iter()
        .copied()
        .filter(|knot| knot.phase.is_finite() && knot.value.is_finite())
        .map(|mut knot| {
            knot.phase = knot.phase.clamp(0.0, 1.0 - MIN_SPACING);
            knot.value = knot.value.clamp(-1.0, 1.0);
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
