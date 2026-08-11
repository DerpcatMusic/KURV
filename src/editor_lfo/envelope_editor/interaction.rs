use super::*;

pub(super) fn envelope_points(
    plot: egui::Rect,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
) -> [egui::Pos2; 5] {
    let weights = [
        envelope_duration_weight(attack),
        envelope_duration_weight(decay),
        ENVELOPE_HOLD_WEIGHT,
        envelope_duration_weight(release),
    ];
    let total: f32 = weights.iter().sum();
    let attack_x = plot.left() + plot.width() * weights[0] / total;
    let decay_x = attack_x + plot.width() * weights[1] / total;
    let sustain_x = decay_x + plot.width() * weights[2] / total;
    let sustain_y = egui::lerp(plot.bottom()..=plot.top(), sustain.clamp(0.0, 1.0));
    [
        plot.left_bottom(),
        egui::pos2(attack_x, plot.top()),
        egui::pos2(decay_x, sustain_y),
        egui::pos2(sustain_x, sustain_y),
        plot.right_bottom(),
    ]
}

pub(super) fn envelope_handles(
    points: &[egui::Pos2; 5],
    curves: [f32; 3],
) -> Vec<(EnvelopeDrag, egui::Pos2)> {
    let mut handles = vec![
        (EnvelopeDrag::Attack, points[1]),
        (EnvelopeDrag::DecaySustain, points[2]),
        (
            EnvelopeDrag::Sustain,
            points[2] + (points[3] - points[2]) * 0.5,
        ),
        (EnvelopeDrag::Release, points[3]),
    ];
    handles.extend([
        (
            EnvelopeDrag::AttackCurve,
            envelope_curve_handle(points[0], points[1], curves[0]),
        ),
        (
            EnvelopeDrag::DecayCurve,
            envelope_curve_handle(points[1], points[2], curves[1]),
        ),
        (
            EnvelopeDrag::ReleaseCurve,
            envelope_curve_handle(points[3], points[4], curves[2]),
        ),
    ]);
    handles
}

pub(super) fn nearest_envelope_target(
    handles: &[(EnvelopeDrag, egui::Pos2)],
    points: &[egui::Pos2; 5],
    curves: [f32; 3],
    pointer: egui::Pos2,
    grab_radius: f32,
) -> Option<EnvelopeDrag> {
    nearest_envelope_handle(&handles[..4], pointer, grab_radius)
        .or_else(|| nearest_envelope_handle(&handles[4..], pointer, grab_radius))
        .or_else(|| {
            [
                EnvelopeDrag::Attack,
                EnvelopeDrag::DecaySustain,
                EnvelopeDrag::Sustain,
                EnvelopeDrag::Release,
            ]
            .into_iter()
            .map(|stage| {
                let (start, end) = envelope_segment(points, stage);
                (
                    stage,
                    distance_to_envelope_stage_sq(
                        pointer,
                        start,
                        end,
                        envelope_curve_for_stage(curves, stage),
                    ),
                )
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= grab_radius.powi(2))
            .map(|(stage, _)| stage)
        })
}

fn nearest_envelope_handle(
    handles: &[(EnvelopeDrag, egui::Pos2)],
    pointer: egui::Pos2,
    grab_radius: f32,
) -> Option<EnvelopeDrag> {
    handles
        .iter()
        .map(|(stage, position)| (*stage, position.distance_sq(pointer)))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, distance)| *distance <= grab_radius.powi(2))
        .map(|(stage, _)| stage)
}

pub(super) fn envelope_stage_label(stage: EnvelopeDrag) -> &'static str {
    match stage {
        EnvelopeDrag::Attack => "ATTACK",
        EnvelopeDrag::AttackCurve => "ATTACK CURVE",
        EnvelopeDrag::DecaySustain => "DECAY + SUSTAIN",
        EnvelopeDrag::DecayCurve => "DECAY CURVE",
        EnvelopeDrag::Sustain => "SUSTAIN",
        EnvelopeDrag::Release => "RELEASE",
        EnvelopeDrag::ReleaseCurve => "RELEASE CURVE",
    }
}

pub(super) fn envelope_segment(
    points: &[egui::Pos2; 5],
    stage: EnvelopeDrag,
) -> (egui::Pos2, egui::Pos2) {
    match stage {
        EnvelopeDrag::Attack | EnvelopeDrag::AttackCurve => (points[0], points[1]),
        EnvelopeDrag::DecaySustain | EnvelopeDrag::DecayCurve => (points[1], points[2]),
        EnvelopeDrag::Sustain => (points[2], points[3]),
        EnvelopeDrag::Release | EnvelopeDrag::ReleaseCurve => (points[3], points[4]),
    }
}

fn envelope_curve_handle(start: egui::Pos2, end: egui::Pos2, curve: f32) -> egui::Pos2 {
    envelope_stage_position(start, end, 0.5, curve)
}

fn envelope_duration_weight(seconds: f32) -> f32 {
    (seconds.max(0.0) + ENVELOPE_TIME_WEIGHT_OFFSET).sqrt()
}

fn envelope_seconds_from_weight(weight: f32) -> f32 {
    (weight.max(ENVELOPE_TIME_WEIGHT_OFFSET.sqrt()).powi(2) - ENVELOPE_TIME_WEIGHT_OFFSET).max(0.0)
}

pub(super) fn envelope_time_at_x(
    stage: EnvelopeDrag,
    pointer_x: f32,
    plot: egui::Rect,
    attack: f32,
    decay: f32,
    release: f32,
) -> f32 {
    let position = ((pointer_x - plot.left()) / plot.width().max(1.0)).clamp(0.001, 0.999);
    let attack_weight = envelope_duration_weight(attack);
    let decay_weight = envelope_duration_weight(decay);
    let release_weight = envelope_duration_weight(release);
    let weight = match stage {
        EnvelopeDrag::Attack => {
            let rest = decay_weight + ENVELOPE_HOLD_WEIGHT + release_weight;
            position * rest / (1.0 - position)
        }
        EnvelopeDrag::DecaySustain => {
            let rest = ENVELOPE_HOLD_WEIGHT + release_weight;
            (position * rest / (1.0 - position) - attack_weight).max(0.0)
        }
        EnvelopeDrag::Release => {
            let before = attack_weight + decay_weight + ENVELOPE_HOLD_WEIGHT;
            before * (1.0 - position) / position
        }
        _ => return 0.0,
    };
    let maximum = if stage == EnvelopeDrag::Release {
        12.0
    } else {
        8.0
    };
    envelope_seconds_from_weight(weight).clamp(0.0, maximum)
}

fn envelope_stage_position(
    start: egui::Pos2,
    end: egui::Pos2,
    progress: f32,
    curve: f32,
) -> egui::Pos2 {
    let shaped = envelope_shaped_progress(progress, curve);
    egui::pos2(
        egui::lerp(start.x..=end.x, progress),
        egui::lerp(start.y..=end.y, shaped),
    )
}

pub(super) fn envelope_path(points: &[egui::Pos2; 5], curves: [f32; 3]) -> Vec<egui::Pos2> {
    let mut path = Vec::with_capacity(ENVELOPE_CURVE_SEGMENTS * 3 + 2);
    append_envelope_stage(&mut path, points[0], points[1], curves[0], true);
    append_envelope_stage(&mut path, points[1], points[2], curves[1], false);
    path.push(points[3]);
    append_envelope_stage(&mut path, points[3], points[4], curves[2], false);
    path
}

pub(super) fn envelope_stage_path(
    start: egui::Pos2,
    end: egui::Pos2,
    curve: f32,
) -> Vec<egui::Pos2> {
    let mut path = Vec::with_capacity(ENVELOPE_CURVE_SEGMENTS + 1);
    append_envelope_stage(&mut path, start, end, curve, true);
    path
}

fn append_envelope_stage(
    path: &mut Vec<egui::Pos2>,
    start: egui::Pos2,
    end: egui::Pos2,
    curve: f32,
    include_start: bool,
) {
    let first = if include_start { 0 } else { 1 };
    for step in first..=ENVELOPE_CURVE_SEGMENTS {
        let progress = step as f32 / ENVELOPE_CURVE_SEGMENTS as f32;
        path.push(envelope_stage_position(start, end, progress, curve));
    }
}

pub(super) fn envelope_curve_for_stage(curves: [f32; 3], stage: EnvelopeDrag) -> f32 {
    match stage {
        EnvelopeDrag::Attack | EnvelopeDrag::AttackCurve => curves[0],
        EnvelopeDrag::DecaySustain | EnvelopeDrag::DecayCurve => curves[1],
        EnvelopeDrag::Sustain => 0.0,
        EnvelopeDrag::Release | EnvelopeDrag::ReleaseCurve => curves[2],
    }
}

fn distance_to_segment_sq(point: egui::Pos2, start: egui::Pos2, end: egui::Pos2) -> f32 {
    let segment = end - start;
    let length_sq = segment.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance_sq(start);
    }
    let position = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance_sq(start + segment * position)
}

fn distance_to_envelope_stage_sq(
    point: egui::Pos2,
    start: egui::Pos2,
    end: egui::Pos2,
    curve: f32,
) -> f32 {
    let mut nearest = f32::INFINITY;
    let mut previous = start;
    for step in 1..=ENVELOPE_CURVE_SEGMENTS {
        let progress = step as f32 / ENVELOPE_CURVE_SEGMENTS as f32;
        let current = envelope_stage_position(start, end, progress, curve);
        nearest = nearest.min(distance_to_segment_sq(point, previous, current));
        previous = current;
    }
    nearest
}
