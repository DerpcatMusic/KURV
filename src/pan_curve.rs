//! Editable per-side unison Shape curves.
//!
//! The editor owns bounded point curves while audio reads fixed runtime
//! segments through a seqlock. Editor vectors and locks never enter audio.

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

pub const MAX_PAN_SHAPE_KNOTS: usize = 4;
pub const MIN_PAN_SHAPE_KNOTS: usize = 2;
const EPSILON: f32 = 1.0e-5;
const HANDLE_X_CENTER: f32 = 0.5;
const HANDLE_X_RANGE: f32 = 0.44;

#[derive(Clone, Copy, Debug, Default, PartialEq, State)]
pub struct PanShapeKnot {
    pub in_lin: f32,
    pub out_lin: f32,
    pub bend_in: f32,
    pub bend_out: f32,
}

#[derive(Clone, Debug, PartialEq, State)]
pub struct PanShapeHalf {
    pub knots: Vec<PanShapeKnot>,
}

impl Default for PanShapeHalf {
    fn default() -> Self {
        Self {
            knots: vec![
                PanShapeKnot {
                    in_lin: 0.0,
                    out_lin: 0.0,
                    bend_out: 0.35,
                    ..PanShapeKnot::default()
                },
                PanShapeKnot {
                    in_lin: 1.0,
                    out_lin: 1.0,
                    ..PanShapeKnot::default()
                },
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, State)]
pub struct PanShapeCurveData {
    pub left: PanShapeHalf,
    pub right: PanShapeHalf,
}

impl Default for PanShapeCurveData {
    fn default() -> Self {
        Self {
            left: PanShapeHalf::default(),
            right: PanShapeHalf::default(),
        }
    }
}

impl PanShapeCurveData {
    pub fn from_legacy(
        center: f32,
        left_edge: f32,
        right_edge: f32,
        left_curve: f32,
        right_curve: f32,
        left_time: f32,
        right_time: f32,
    ) -> Self {
        let mut data = Self::default();
        for (half, edge, curve, time) in [
            (&mut data.left, left_edge, left_curve, left_time),
            (&mut data.right, right_edge, right_curve, right_time),
        ] {
            half.knots[0].out_lin = center.clamp(0.0, 1.0);
            half.knots[1].out_lin = edge.clamp(0.0, 1.0);
            // Preserve the legacy curve-time as the draggable point's X
            // coordinate rather than discarding it as an old control value.
            let horizontal =
                ((time.clamp(0.05, 0.95) - HANDLE_X_CENTER) / HANDLE_X_RANGE).clamp(-1.0, 1.0);
            set_segment_curve(half, 0, curve, horizontal);
        }
        data
    }

    pub fn half(&self, left: bool) -> &PanShapeHalf {
        if left { &self.left } else { &self.right }
    }

    pub fn half_mut(&mut self, left: bool) -> &mut PanShapeHalf {
        if left {
            &mut self.left
        } else {
            &mut self.right
        }
    }

    pub fn compile_rt(&self) -> (PanShapeSegmentsRt, PanShapeSegmentsRt) {
        (self.left.compile_rt(), self.right.compile_rt())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PanShapeSegmentsRt {
    pub count: u8,
    pub seg_x0: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_x1: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_p0: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_p1: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_p2: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_p3: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_cx1: [f32; MAX_PAN_SHAPE_KNOTS - 1],
    pub seg_cx2: [f32; MAX_PAN_SHAPE_KNOTS - 1],
}

impl PanShapeHalf {
    pub fn compile_rt(&self) -> PanShapeSegmentsRt {
        let knots = sanitize_knots(&self.knots);
        let mut rt = PanShapeSegmentsRt::default();
        if knots.len() < MIN_PAN_SHAPE_KNOTS {
            return rt;
        }
        rt.count = (knots.len() - 1) as u8;
        for (index, pair) in knots.windows(2).enumerate() {
            let [k0, k1] = [pair[0], pair[1]];
            rt.seg_x0[index] = k0.in_lin;
            rt.seg_x1[index] = k1.in_lin;
            rt.seg_p0[index] = k0.out_lin;
            // `seg_p1`/`seg_cx1` are the normalized point dragged in the
            // editor. The remaining slots stay populated so the fixed-size
            // snapshot layout remains stable.
            rt.seg_p1[index] = (0.5 + k0.bend_out.clamp(-1.0, 1.0) * 0.5).clamp(0.0, 1.0);
            rt.seg_p2[index] = k1.out_lin;
            rt.seg_p3[index] = k1.out_lin;
            rt.seg_cx1[index] = (HANDLE_X_CENTER + k1.bend_in.clamp(-1.0, 1.0) * HANDLE_X_RANGE)
                .clamp(EPSILON, 1.0 - EPSILON);
            rt.seg_cx2[index] = 1.0;
        }
        rt
    }
}

impl PanShapeSegmentsRt {
    pub const fn identity() -> Self {
        let mut result = Self {
            count: 1,
            ..Self::zeroed()
        };
        result.seg_x0[0] = 0.0;
        result.seg_x1[0] = 1.0;
        result.seg_p0[0] = 0.0;
        result.seg_p1[0] = 0.5;
        result.seg_p2[0] = 1.0;
        result.seg_p3[0] = 1.0;
        result.seg_cx1[0] = 0.5;
        result.seg_cx2[0] = 1.0;
        result
    }

    const fn zeroed() -> Self {
        Self {
            count: 0,
            seg_x0: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_x1: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_p0: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_p1: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_p2: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_p3: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_cx1: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
            seg_cx2: [0.0; MAX_PAN_SHAPE_KNOTS - 1],
        }
    }

    #[inline]
    pub fn eval(&self, input: f32) -> f32 {
        self.eval_point(input)
    }

    /// Realtime path: bounded, allocation-free interpolation through the
    /// exact editor point. It is the same smooth curve the editor renders.
    #[inline]
    pub fn eval_fast(&self, input: f32) -> f32 {
        self.eval_point(input)
    }

    #[inline]
    fn eval_point(&self, input: f32) -> f32 {
        let input = bounded(input);
        if self.count == 0 {
            return input;
        }
        let last = usize::from(self.count - 1);
        if input <= self.seg_x0[0] {
            return bounded(self.seg_p0[0]);
        }
        if input >= self.seg_x1[last] {
            return bounded(self.seg_p3[last]);
        }
        for index in 0..usize::from(self.count) {
            if input <= self.seg_x1[index] || index == last {
                let x0 = self.seg_x0[index];
                let x1 = self.seg_x1[index];
                if !x0.is_finite() || !x1.is_finite() || x1 <= x0 {
                    return bounded(self.seg_p0[index]);
                }
                let target = ((input - x0) / (x1 - x0)).clamp(0.0, 1.0);
                let handle_x = bounded(self.seg_cx1[index]).clamp(EPSILON, 1.0 - EPSILON);
                let start = bounded(self.seg_p0[index]);
                let handle = bounded(self.seg_p1[index]);
                let end = bounded(self.seg_p3[index]);
                let warped = schlick_bias(target, 1.0 - handle_x);
                let shaped = schlick_bias(warped, handle);
                return bounded((end - start).mul_add(shaped, start));
            }
        }
        input
    }
}

struct AtomicPanShapeSegments {
    count: AtomicU32,
    x0: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    x1: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    p0: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    p1: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    p2: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    p3: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    cx1: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
    cx2: [AtomicU32; MAX_PAN_SHAPE_KNOTS - 1],
}

impl AtomicPanShapeSegments {
    fn new(segments: PanShapeSegmentsRt) -> Self {
        let result = Self {
            count: AtomicU32::new(0),
            x0: std::array::from_fn(|_| AtomicU32::new(0)),
            x1: std::array::from_fn(|_| AtomicU32::new(0)),
            p0: std::array::from_fn(|_| AtomicU32::new(0)),
            p1: std::array::from_fn(|_| AtomicU32::new(0)),
            p2: std::array::from_fn(|_| AtomicU32::new(0)),
            p3: std::array::from_fn(|_| AtomicU32::new(0)),
            cx1: std::array::from_fn(|_| AtomicU32::new(0)),
            cx2: std::array::from_fn(|_| AtomicU32::new(0)),
        };
        result.store(segments);
        result
    }

    fn store(&self, segments: PanShapeSegmentsRt) {
        for index in 0..MAX_PAN_SHAPE_KNOTS - 1 {
            self.x0[index].store(segments.seg_x0[index].to_bits(), Ordering::Relaxed);
            self.x1[index].store(segments.seg_x1[index].to_bits(), Ordering::Relaxed);
            self.p0[index].store(segments.seg_p0[index].to_bits(), Ordering::Relaxed);
            self.p1[index].store(segments.seg_p1[index].to_bits(), Ordering::Relaxed);
            self.p2[index].store(segments.seg_p2[index].to_bits(), Ordering::Relaxed);
            self.p3[index].store(segments.seg_p3[index].to_bits(), Ordering::Relaxed);
            self.cx1[index].store(segments.seg_cx1[index].to_bits(), Ordering::Relaxed);
            self.cx2[index].store(segments.seg_cx2[index].to_bits(), Ordering::Relaxed);
        }
        self.count
            .store(u32::from(segments.count), Ordering::Relaxed);
    }

    fn load(&self) -> PanShapeSegmentsRt {
        let count = self
            .count
            .load(Ordering::Relaxed)
            .min((MAX_PAN_SHAPE_KNOTS - 1) as u32) as u8;
        let mut result = PanShapeSegmentsRt {
            count,
            ..PanShapeSegmentsRt::default()
        };
        for index in 0..usize::from(count) {
            result.seg_x0[index] = f32::from_bits(self.x0[index].load(Ordering::Relaxed));
            result.seg_x1[index] = f32::from_bits(self.x1[index].load(Ordering::Relaxed));
            result.seg_p0[index] = f32::from_bits(self.p0[index].load(Ordering::Relaxed));
            result.seg_p1[index] = f32::from_bits(self.p1[index].load(Ordering::Relaxed));
            result.seg_p2[index] = f32::from_bits(self.p2[index].load(Ordering::Relaxed));
            result.seg_p3[index] = f32::from_bits(self.p3[index].load(Ordering::Relaxed));
            result.seg_cx1[index] = f32::from_bits(self.cx1[index].load(Ordering::Relaxed));
            result.seg_cx2[index] = f32::from_bits(self.cx2[index].load(Ordering::Relaxed));
        }
        result
    }
}

pub struct PanShapeCurveState {
    data: RwLock<PanShapeCurveData>,
    generation: AtomicU32,
    initialized: AtomicBool,
    left_rt: AtomicPanShapeSegments,
    right_rt: AtomicPanShapeSegments,
}

impl PanShapeCurveState {
    pub fn new() -> Self {
        let data = PanShapeCurveData::default();
        let (left, right) = data.compile_rt();
        Self {
            data: RwLock::new(data),
            generation: AtomicU32::new(0),
            initialized: AtomicBool::new(false),
            left_rt: AtomicPanShapeSegments::new(left),
            right_rt: AtomicPanShapeSegments::new(right),
        }
    }

    pub fn snapshot(&self) -> PanShapeCurveData {
        match self.data.read() {
            Ok(data) => data.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    #[must_use]
    pub(crate) fn history_generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    pub fn replace(&self, data: PanShapeCurveData) {
        let sanitized = sanitize_data(data);
        let (left, right) = sanitized.compile_rt();
        match self.data.write() {
            Ok(mut current) => *current = sanitized,
            Err(poisoned) => *poisoned.into_inner() = sanitized,
        }
        self.publish(left, right);
        self.initialized.store(true, Ordering::Release);
    }

    pub fn edit<R>(&self, edit: impl FnOnce(&mut PanShapeCurveData) -> R) -> R {
        let mut guard = match self.data.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let result = edit(&mut guard);
        *guard = sanitize_data(std::mem::take(&mut *guard));
        let (left, right) = guard.compile_rt();
        drop(guard);
        self.publish(left, right);
        self.initialized.store(true, Ordering::Release);
        result
    }

    pub fn segments_rt(&self) -> (PanShapeSegmentsRt, PanShapeSegmentsRt) {
        for _ in 0..3 {
            if let Some(result) = self.try_segments_rt() {
                return result;
            }
        }
        (
            PanShapeSegmentsRt::identity(),
            PanShapeSegmentsRt::identity(),
        )
    }

    pub fn try_segments_rt(&self) -> Option<(PanShapeSegmentsRt, PanShapeSegmentsRt)> {
        self.try_segments_rt_after(u32::MAX)
            .map(|(_, segments)| segments)
    }

    pub fn try_segments_rt_after(
        &self,
        observed_generation: u32,
    ) -> Option<(u32, (PanShapeSegmentsRt, PanShapeSegmentsRt))> {
        let before = self.generation.load(Ordering::Acquire);
        if before == observed_generation || before & 1 != 0 {
            return None;
        }
        let left = self.left_rt.load();
        let right = self.right_rt.load();
        (self.generation.load(Ordering::Acquire) == before).then_some((before, (left, right)))
    }

    fn publish(&self, left: PanShapeSegmentsRt, right: PanShapeSegmentsRt) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.left_rt.store(left);
        self.right_rt.store(right);
        self.generation.fetch_add(1, Ordering::Release);
    }
}

impl Default for PanShapeCurveState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for PanShapeCurveState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        self.snapshot().write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        if let Some(data) = PanShapeCurveData::read_field(cursor) {
            self.replace(data);
        }
    }
}

pub fn move_endpoint(half: &mut PanShapeHalf, output: f32) -> bool {
    let Some(knot) = half.knots.last_mut() else {
        return false;
    };
    knot.out_lin = output.clamp(0.0, 1.0);
    true
}

pub fn move_center(data: &mut PanShapeCurveData, output: f32) -> bool {
    let output = output.clamp(0.0, 1.0);
    let mut changed = false;
    if let Some(knot) = data.left.knots.first_mut() {
        knot.out_lin = output;
        changed = true;
    }
    if let Some(knot) = data.right.knots.first_mut() {
        knot.out_lin = output;
        changed = true;
    }
    changed
}

pub fn insert_knot(half: &mut PanShapeHalf, input: f32, output: f32) -> bool {
    if half.knots.len() >= MAX_PAN_SHAPE_KNOTS {
        return false;
    }
    let input = input.clamp(EPSILON, 1.0 - EPSILON);
    if half
        .knots
        .iter()
        .any(|knot| (knot.in_lin - input).abs() <= EPSILON * 2.0)
    {
        return false;
    }
    half.knots.push(PanShapeKnot {
        in_lin: input,
        out_lin: output.clamp(0.0, 1.0),
        ..PanShapeKnot::default()
    });
    half.knots
        .sort_by(|left, right| left.in_lin.total_cmp(&right.in_lin));
    true
}

pub fn move_knot(half: &mut PanShapeHalf, index: usize, input: f32, output: f32) -> bool {
    if index == 0 || index + 1 >= half.knots.len() {
        return false;
    }
    let minimum = half.knots[index - 1].in_lin + EPSILON;
    let maximum = half.knots[index + 1].in_lin - EPSILON;
    if minimum > maximum {
        return false;
    }
    half.knots[index].in_lin = input.clamp(minimum, maximum);
    half.knots[index].out_lin = output.clamp(0.0, 1.0);
    true
}

pub fn remove_knot(half: &mut PanShapeHalf, index: usize) -> bool {
    if index == 0 || index + 1 >= half.knots.len() || half.knots.len() <= MIN_PAN_SHAPE_KNOTS {
        return false;
    }
    half.knots.remove(index);
    true
}

pub fn set_segment_curve(
    half: &mut PanShapeHalf,
    index: usize,
    vertical: f32,
    horizontal: f32,
) -> bool {
    if index + 1 >= half.knots.len() {
        return false;
    }
    half.knots[index].bend_out = vertical.clamp(-1.0, 1.0);
    half.knots[index + 1].bend_in = horizontal.clamp(-1.0, 1.0);
    true
}

#[inline]
fn bounded(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[inline]
fn schlick_bias(value: f32, bias: f32) -> f32 {
    let bias = bias.clamp(0.005, 0.995);
    value / ((bias.recip() - 2.0).mul_add(1.0 - value, 1.0))
}

fn sanitize_data(mut data: PanShapeCurveData) -> PanShapeCurveData {
    data.left.knots = sanitize_knots(&data.left.knots);
    data.right.knots = sanitize_knots(&data.right.knots);
    data
}

fn sanitize_knots(knots: &[PanShapeKnot]) -> Vec<PanShapeKnot> {
    let mut result = knots
        .iter()
        .copied()
        .filter(|knot| knot.in_lin.is_finite() && knot.out_lin.is_finite())
        .map(|mut knot| {
            knot.in_lin = knot.in_lin.clamp(0.0, 1.0);
            knot.out_lin = knot.out_lin.clamp(0.0, 1.0);
            knot.bend_in = knot.bend_in.clamp(-1.0, 1.0);
            knot.bend_out = knot.bend_out.clamp(-1.0, 1.0);
            knot
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| a.in_lin.total_cmp(&b.in_lin));
    result.dedup_by(|left, right| (left.in_lin - right.in_lin).abs() <= EPSILON * 2.0);
    if result.len() > MAX_PAN_SHAPE_KNOTS {
        result.truncate(MAX_PAN_SHAPE_KNOTS);
    }
    if result.len() < MIN_PAN_SHAPE_KNOTS {
        return PanShapeHalf::default().knots;
    }
    result[0].in_lin = 0.0;
    if let Some(last) = result.last_mut() {
        last.in_lin = 1.0;
    }
    result
}
