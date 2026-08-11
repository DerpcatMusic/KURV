use super::*;

#[derive(Clone, Copy)]
pub(super) struct SplineGeometry {
    plot: egui::Rect,
    bipolar: bool,
}

impl SplineGeometry {
    pub(super) fn new(plot: egui::Rect, bipolar: bool) -> Self {
        Self { plot, bipolar }
    }

    pub(super) fn plot(self) -> egui::Rect {
        self.plot
    }

    pub(super) fn bipolar(self) -> bool {
        self.bipolar
    }

    pub(super) fn position(self, phase: f32, value: f32) -> egui::Pos2 {
        let y = if self.bipolar {
            (-value * self.plot.height() * 0.42).mul_add(1.0, self.plot.center().y)
        } else {
            self.plot.bottom() - value.mul_add(0.5, 0.5) * self.plot.height() * 0.9
        };
        egui::pos2(phase.mul_add(self.plot.width(), self.plot.left()), y)
    }

    pub(super) fn values_from_pos(self, position: egui::Pos2) -> (f32, f32) {
        let phase = ((position.x - self.plot.left()) / self.plot.width()).clamp(0.0, 1.0);
        let value = if self.bipolar {
            (self.plot.center().y - position.y) / (self.plot.height() * 0.42)
        } else {
            ((self.plot.bottom() - position.y) / (self.plot.height() * 0.9)).mul_add(2.0, -1.0)
        }
        .clamp(-1.0, 1.0);
        (phase, value)
    }

    pub(super) fn nearest_target(
        self,
        data: &WaveCurveData,
        pointer: egui::Pos2,
        point_radius: f32,
        grab_radius: f32,
    ) -> Option<SplineDrag> {
        let grab_radius_sq = grab_radius.powi(2);
        let point = data
            .knots
            .iter()
            .enumerate()
            .map(|(index, knot)| {
                (
                    SplineDrag::Point(index),
                    self.position(knot.phase, knot.value).distance_sq(pointer),
                )
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= grab_radius_sq)
            .map(|(target, _)| target);
        if point.is_some() {
            return point;
        }

        let handle = segment_handles(data, self, point_radius)
            .map(|handle| {
                (
                    SplineDrag::Tension(handle.index),
                    handle.position.distance_sq(pointer),
                )
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= grab_radius_sq)
            .map(|(target, _)| target);
        if handle.is_some() {
            return handle;
        }

        let phase = ((pointer.x - self.plot.left()) / self.plot.width().max(1.0)).clamp(0.0, 1.0);
        let segment = data
            .knots
            .partition_point(|knot| knot.phase <= phase)
            .saturating_sub(1)
            .min(data.knots.len().saturating_sub(1));
        (self
            .position(phase, curve_value(data, phase))
            .distance_sq(pointer)
            <= grab_radius_sq)
            .then_some(SplineDrag::Tension(segment))
    }

    pub(super) fn snap_point(
        self,
        phase: f32,
        value: f32,
        point_radius: f32,
        disabled: bool,
    ) -> (f32, f32, Option<f32>, Option<f32>) {
        if disabled {
            return (phase, value, None, None);
        }
        let proximity = point_radius * 1.5;
        let snap_phase = [0.25_f32, 0.5, 0.75]
            .into_iter()
            .map(|target| (target, (target - phase).abs() * self.plot.width()))
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= proximity)
            .map(|(target, _)| target);
        let value_scale = self.plot.height() * if self.bipolar { 0.42 } else { 0.45 };
        let snap_value = [-1.0_f32, -0.5, 0.0, 0.5, 1.0]
            .into_iter()
            .map(|target| (target, (target - value).abs() * value_scale))
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= proximity)
            .map(|(target, _)| target);
        (
            snap_phase.unwrap_or(phase),
            snap_value.unwrap_or(value),
            snap_phase,
            snap_value,
        )
    }
}

#[derive(Clone, Copy)]
pub(super) struct SegmentHandle {
    pub(super) index: usize,
    pub(super) position: egui::Pos2,
}

pub(super) fn segment_handles(
    data: &WaveCurveData,
    geometry: SplineGeometry,
    point_radius: f32,
) -> impl Iterator<Item = SegmentHandle> + '_ {
    data.knots
        .iter()
        .enumerate()
        .filter_map(move |(index, knot)| {
            let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
            let phase = segment_handle_phase(data, index)?;
            ((end - knot.phase) * geometry.plot().width() >= point_radius * 4.0).then(|| {
                let value = curve_value(data, phase);
                SegmentHandle {
                    index,
                    position: geometry.position(phase, value),
                }
            })
        })
}

pub(super) fn curve_value(data: &WaveCurveData, phase: f32) -> f32 {
    let count = data.knots.len();
    if count < MIN_WAVE_KNOTS {
        return 0.0;
    }
    let phase = phase.rem_euclid(1.0);
    let index = data
        .knots
        .partition_point(|knot| knot.phase <= phase)
        .saturating_sub(1)
        .min(count - 1);
    let width = segment_width(data, index);
    let progress = ((phase - data.knots[index].phase) / width).clamp(0.0, 1.0);
    let shaped =
        shape_segment_progress(progress, data.knots[index].curve, data.knots[index].curve_x);
    segment_value_at_progress(data, index, shaped)
}

pub(super) fn segment_curve_for_value(data: &WaveCurveData, index: usize, value: f32) -> f32 {
    let low_value = segment_value_at_progress(data, index, 0.0);
    let high_value = segment_value_at_progress(data, index, 1.0);
    if (high_value - low_value).abs() <= f32::EPSILON {
        return data.knots.get(index).map_or(0.0, |knot| knot.curve);
    }
    let target = value.clamp(low_value.min(high_value), low_value.max(high_value));
    let ascending = high_value > low_value;
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..16 {
        let progress = (low + high) * 0.5;
        let candidate = segment_value_at_progress(data, index, progress);
        if (candidate < target) == ascending {
            low = progress;
        } else {
            high = progress;
        }
    }
    // At the visual handle midpoint the shaping equation is
    // `0.5 + curve * 0.25`, so its inverse is `curve = 4p - 2`.
    // Using `2p - 2` made the neutral midpoint resolve to -1 and caused bends
    // to jump away from the pointer as soon as a drag began.
    (low + high).mul_add(4.0, -2.0)
}

fn segment_value_at_progress(data: &WaveCurveData, index: usize, progress: f32) -> f32 {
    let count = data.knots.len();
    let next = (index + 1) % count;
    let width = segment_width(data, index);
    let m0 = curve_tangent(data, index) * width;
    let m1 = curve_tangent(data, next) * width;
    let start = data.knots[index].value;
    let end = data.knots[next].value;
    ((2.0 * start - 2.0 * end + m0 + m1) * progress + (-3.0 * start + 3.0 * end - 2.0 * m0 - m1))
        * progress
        * progress
        + m0 * progress
        + start
}

fn segment_width(data: &WaveCurveData, index: usize) -> f32 {
    let next = (index + 1) % data.knots.len();
    let end = if next == 0 {
        1.0
    } else {
        data.knots[next].phase
    };
    (end - data.knots[index].phase).max(f32::EPSILON)
}

fn curve_tangent(data: &WaveCurveData, index: usize) -> f32 {
    let count = data.knots.len();
    let previous = (index + count - 1) % count;
    let before =
        (data.knots[index].value - data.knots[previous].value) / segment_width(data, previous);
    let after = (data.knots[(index + 1) % count].value - data.knots[index].value)
        / segment_width(data, index);
    if before * after <= 0.0 {
        return 0.0;
    }
    let before_weight = 2.0 * segment_width(data, index) + segment_width(data, previous);
    let after_weight = segment_width(data, index) + 2.0 * segment_width(data, previous);
    (before_weight + after_weight) / (before_weight / before + after_weight / after)
}

pub(super) fn nearest_knot(data: &WaveCurveData, phase: f32) -> Option<usize> {
    data.knots
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            (left.phase - phase)
                .abs()
                .total_cmp(&(right.phase - phase).abs())
        })
        .map(|(index, _)| index)
}
