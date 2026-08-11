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
        compiled: WaveCurveRt,
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

        let handle = segment_handles(data, compiled, self, point_radius)
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
            .position(phase, compiled.eval(phase))
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
    compiled: WaveCurveRt,
    geometry: SplineGeometry,
    point_radius: f32,
) -> impl Iterator<Item = SegmentHandle> + '_ {
    data.knots
        .iter()
        .enumerate()
        .filter_map(move |(index, knot)| {
            let end = data.knots.get(index + 1).map_or(1.0, |next| next.phase);
            let phase = (knot.phase + end) * 0.5;
            ((end - knot.phase) * geometry.plot().width() >= point_radius * 4.0).then(|| {
                let value = compiled.eval(phase);
                SegmentHandle {
                    index,
                    position: geometry.position(phase, value),
                }
            })
        })
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
