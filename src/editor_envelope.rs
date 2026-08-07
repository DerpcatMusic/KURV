//! ADSR and expression envelope editor.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::{KurvParams, P, editor_modulation, editor_theme, editor_widgets};

const CURVE_POINTS: u16 = 96;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvelopeHandle {
    Attack,
    DecaySustain,
    Release,
    AttackCurve,
    DecayCurve,
    ReleaseCurve,
}

#[derive(Clone, Copy)]
struct EnvelopeDrag {
    handle: EnvelopeHandle,
    value: f32,
    secondary_value: f32,
}

impl EnvelopeHandle {
    const fn param(self) -> P {
        match self {
            Self::Attack => P::Attack,
            Self::DecaySustain => P::Decay,
            Self::Release => P::Release,
            Self::AttackCurve => P::AttackCurve,
            Self::DecayCurve => P::DecayCurve,
            Self::ReleaseCurve => P::ReleaseCurve,
        }
    }

    const fn secondary_param(self) -> Option<P> {
        match self {
            Self::DecaySustain => Some(P::Sustain),
            Self::AttackCurve => Some(P::AttackCurveTime),
            Self::DecayCurve => Some(P::DecayCurveTime),
            Self::ReleaseCurve => Some(P::ReleaseCurveTime),
            Self::Attack | Self::Release => None,
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    reason = "the envelope timeline, handles, host gestures, and labels share one coordinate system"
)]
pub(crate) fn envelope_view(ui: &mut egui::Ui, state: &PluginContext<KurvParams>, height: f32) {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
    let rect = response.rect;
    let graph =
        editor_widgets::graph_plot(rect, ui, editor_theme::space::SM, editor_theme::space::SM);
    let top = graph.top() + editor_theme::space::XS;
    let bottom = graph.bottom() - editor_theme::metrics(ui).points(1.5).clamp(20.0, 26.0);
    let width = graph.width();

    let attack = editor_modulation::effective_plain_value(state, P::Attack);
    let decay = editor_modulation::effective_plain_value(state, P::Decay);
    let sustain = editor_modulation::effective_plain_value(state, P::Sustain);
    let release = editor_modulation::effective_plain_value(state, P::Release);
    let attack_curve = editor_modulation::effective_normalized(state, P::AttackCurve);
    let decay_curve = editor_modulation::effective_normalized(state, P::DecayCurve);
    let release_curve = editor_modulation::effective_normalized(state, P::ReleaseCurve);
    let attack_curve_time = editor_modulation::effective_normalized(state, P::AttackCurveTime);
    let decay_curve_time = editor_modulation::effective_normalized(state, P::DecayCurveTime);
    let release_curve_time = editor_modulation::effective_normalized(state, P::ReleaseCurveTime);

    // Sustain is deliberately untimed. A/D/R share a 500 ms minimum
    // horizon that expands continuously instead of snapping between scales.
    let sustain_gap = (width * 0.16).clamp(
        editor_theme::metrics(ui).points(5.0),
        editor_theme::metrics(ui).points(9.0),
    );
    let timed_width = (width - sustain_gap).max(1.0);
    let horizon = envelope_horizon(attack + decay + release);
    let pixels_per_second = timed_width / horizon;
    let stage_width = |seconds: f32| {
        if seconds <= f32::EPSILON {
            12.0
        } else {
            (seconds * pixels_per_second).max(18.0)
        }
    };
    let stage_widths = [
        stage_width(attack),
        stage_width(decay),
        stage_width(release),
    ];
    let attack_x = graph.left() + stage_widths[0];
    let decay_x = attack_x + stage_widths[1];
    let release_x = decay_x + sustain_gap;
    let drag_id = response.id.with("envelope_handle");
    let start = egui::pos2(graph.left(), bottom);
    let attack_end = egui::pos2(attack_x, top);
    let sustain_y = (top - bottom).mul_add(sustain, bottom);
    let decay_end = egui::pos2(decay_x, sustain_y);
    let release_start = egui::pos2(release_x, sustain_y);
    let release_end = egui::pos2((release_x + stage_widths[2]).min(graph.right()), bottom);
    let attack_curve_point =
        curve_handle_position(start, attack_end, attack_curve_time, attack_curve);
    let decay_curve_point =
        curve_handle_position(attack_end, decay_end, decay_curve_time, decay_curve);
    let release_curve_point = curve_handle_position(
        release_start,
        release_end,
        release_curve_time,
        release_curve,
    );
    let handles = [
        (attack_end, EnvelopeHandle::Attack),
        (decay_end, EnvelopeHandle::DecaySustain),
        (release_end, EnvelopeHandle::Release),
        (attack_curve_point, EnvelopeHandle::AttackCurve),
        (decay_curve_point, EnvelopeHandle::DecayCurve),
        (release_curve_point, EnvelopeHandle::ReleaseCurve),
    ];
    let curve_segment = |handle| match handle {
        EnvelopeHandle::AttackCurve => Some((start, attack_end)),
        EnvelopeHandle::DecayCurve => Some((attack_end, decay_end)),
        EnvelopeHandle::ReleaseCurve => Some((release_start, release_end)),
        EnvelopeHandle::Attack | EnvelopeHandle::DecaySustain | EnvelopeHandle::Release => None,
    };
    let attack_zone = destination_zone(ui, response.id.with("attack-destination"), attack_end);
    let decay_zone = destination_zone(ui, response.id.with("decay-destination"), decay_end);
    let release_zone = destination_zone(ui, response.id.with("release-destination"), release_end);
    let attack_curve_zone = destination_zone(
        ui,
        response.id.with("attack-curve-destination"),
        attack_curve_point,
    );
    let decay_curve_zone = destination_zone(
        ui,
        response.id.with("decay-curve-destination"),
        decay_curve_point,
    );
    let release_curve_zone = destination_zone(
        ui,
        response.id.with("release-curve-destination"),
        release_curve_point,
    );
    let modulation_gesture = editor_modulation::owns_gesture(ui, state, P::Attack, &response)
        || editor_modulation::owns_gesture(ui, state, P::Decay, &response)
        || editor_modulation::owns_gesture(ui, state, P::Sustain, &response)
        || editor_modulation::owns_gesture(ui, state, P::Release, &response)
        || editor_modulation::owns_gesture(ui, state, P::AttackCurve, &response)
        || editor_modulation::owns_gesture(ui, state, P::AttackCurveTime, &response)
        || editor_modulation::owns_gesture(ui, state, P::DecayCurve, &response)
        || editor_modulation::owns_gesture(ui, state, P::DecayCurveTime, &response)
        || editor_modulation::owns_gesture(ui, state, P::ReleaseCurve, &response)
        || editor_modulation::owns_gesture(ui, state, P::ReleaseCurveTime, &response);
    if !modulation_gesture
        && response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some((_, handle)) = handles
            .iter()
            .filter(|(position, _)| position.distance_sq(pointer) <= 22.0_f32.powi(2))
            .min_by(|(left, _), (right, _)| {
                left.distance_sq(pointer)
                    .total_cmp(&right.distance_sq(pointer))
            })
    {
        state.begin_edit(handle.param());
        if let Some(param) = handle.secondary_param() {
            state.begin_edit(param);
        }
        ui.data_mut(|data| {
            data.insert_temp(
                drag_id,
                EnvelopeDrag {
                    handle: *handle,
                    value: state.get_param(handle.param()),
                    secondary_value: handle
                        .secondary_param()
                        .map_or(0.0, |param| state.get_param(param)),
                },
            );
        });
    }
    if !modulation_gesture
        && response.dragged()
        && let Some(mut drag) = ui.data_mut(|data| data.get_temp::<EnvelopeDrag>(drag_id))
    {
        let motion = response.drag_motion();
        if drag.handle == EnvelopeHandle::DecaySustain {
            if let Some(info) = state
                .params()
                .param_infos()
                .into_iter()
                .find(|info| info.id == u32::from(P::Decay))
            {
                let plain = info.range.denormalize(f64::from(drag.value));
                let delta_seconds = motion.x * horizon / timed_width;
                drag.value = info.range.normalize(plain + f64::from(delta_seconds)) as f32;
            }
            drag.secondary_value =
                (drag.secondary_value - motion.y / graph.height().max(80.0)).clamp(0.0, 1.0);
            state.set_param(P::Sustain, f64::from(drag.secondary_value));
        } else if let Some((curve_start, curve_end)) = curve_segment(drag.handle) {
            let horizontal_span = (curve_end.x - curve_start.x).abs().max(24.0);
            let vertical_span = curve_end.y - curve_start.y;
            let vertical_span = if vertical_span.abs() >= 16.0 {
                vertical_span
            } else if drag.handle == EnvelopeHandle::AttackCurve {
                -graph.height()
            } else {
                graph.height()
            };
            drag.secondary_value =
                (drag.secondary_value + motion.x / horizontal_span).clamp(0.0, 1.0);
            drag.value = (drag.value + motion.y / vertical_span).clamp(0.0, 1.0);
            if let Some(param) = drag.handle.secondary_param() {
                state.set_param(param, f64::from(drag.secondary_value));
            }
        } else if let Some(info) = state
            .params()
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(drag.handle.param()))
        {
            let plain = info.range.denormalize(f64::from(drag.value));
            let delta_seconds = motion.x * horizon / timed_width;
            drag.value = info.range.normalize(plain + f64::from(delta_seconds)) as f32;
        }
        ui.data_mut(|data| data.insert_temp(drag_id, drag));
        state.set_param(drag.handle.param(), f64::from(drag.value));
    }
    if !modulation_gesture
        && response.drag_stopped()
        && let Some(drag) = ui.data_mut(|data| {
            let drag = data.get_temp::<EnvelopeDrag>(drag_id);
            data.remove::<EnvelopeDrag>(drag_id);
            drag
        })
    {
        state.end_edit(drag.handle.param());
        if let Some(param) = drag.handle.secondary_param() {
            state.end_edit(param);
        }
    }

    editor_widgets::graph_frame(&painter, rect);
    editor_widgets::graph_grid(&painter, graph, 4, 4);
    let envelope_color = editor_theme::semantic().envelope;
    let attack_color = envelope_color;
    let decay_color = envelope_color.linear_multiply(0.82);
    let release_color = envelope_color;
    let decay_sustain_color = envelope_color;

    for segment in 0..8_u8 {
        if segment.is_multiple_of(2) {
            let y0 = egui::lerp(top..=bottom, f32::from(segment) / 8.0);
            let y1 = egui::lerp(top..=bottom, f32::from(segment + 1) / 8.0);
            painter.line_segment(
                [
                    egui::pos2(release_start.x, y0),
                    egui::pos2(release_start.x, y1),
                ],
                egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
            );
        }
    }

    let mut attack_points = Vec::with_capacity(usize::from(CURVE_POINTS + 1));
    let mut decay_points = Vec::with_capacity(usize::from(CURVE_POINTS + 1));
    let mut release_points = Vec::with_capacity(usize::from(CURVE_POINTS + 1));
    push_envelope_curve(
        &mut attack_points,
        start,
        attack_end,
        attack_curve_time,
        attack_curve,
    );
    push_envelope_curve(
        &mut decay_points,
        attack_end,
        decay_end,
        decay_curve_time,
        decay_curve,
    );
    push_envelope_curve(
        &mut release_points,
        release_start,
        release_end,
        release_curve_time,
        release_curve,
    );
    let sustain_points = [decay_end, release_start];
    let envelope_fill = envelope_color;
    editor_widgets::gradient_area_to_bottom(&painter, &attack_points, bottom, envelope_fill, 38);
    editor_widgets::gradient_area_to_bottom(&painter, &decay_points, bottom, envelope_fill, 38);
    editor_widgets::gradient_area_to_bottom(&painter, &sustain_points, bottom, envelope_fill, 38);
    editor_widgets::gradient_area_to_bottom(&painter, &release_points, bottom, envelope_fill, 38);
    painter.add(egui::Shape::line(
        attack_points,
        egui::Stroke::new(1.8_f32, attack_color),
    ));
    painter.add(egui::Shape::line(
        decay_points,
        egui::Stroke::new(1.8_f32, decay_color),
    ));
    painter.line_segment(
        [decay_end, release_start],
        egui::Stroke::new(1.8_f32, decay_color),
    );
    painter.add(egui::Shape::line(
        release_points,
        egui::Stroke::new(1.8_f32, release_color),
    ));
    if release_end.x < graph.right() {
        painter.line_segment(
            [release_end, egui::pos2(graph.right(), bottom)],
            egui::Stroke::new(1.0_f32, editor_theme::semantic().grid),
        );
    }

    let break_x = decay_end.x + sustain_gap * 0.58;
    for offset in [-4.0_f32, 4.0] {
        painter.line_segment(
            [
                egui::pos2(break_x + offset - 3.0, sustain_y + 6.0),
                egui::pos2(break_x + offset + 3.0, sustain_y - 6.0),
            ],
            egui::Stroke::new(3.0_f32, editor_theme::semantic().surface),
        );
        painter.line_segment(
            [
                egui::pos2(break_x + offset - 3.0, sustain_y + 6.0),
                egui::pos2(break_x + offset + 3.0, sustain_y - 6.0),
            ],
            egui::Stroke::new(1.3_f32, decay_color),
        );
    }

    let active_handle = ui
        .data_mut(|data| data.get_temp::<EnvelopeDrag>(drag_id))
        .map(|drag| drag.handle);
    let hover = response.hover_pos();
    for (position, handle) in handles {
        let is_curve = matches!(
            handle,
            EnvelopeHandle::AttackCurve | EnvelopeHandle::DecayCurve | EnvelopeHandle::ReleaseCurve
        );
        let highlighted = active_handle == Some(handle)
            || hover.is_some_and(|pointer| pointer.distance(position) < 11.0);
        let radius = if is_curve { 3.5 } else { 4.0 };
        let handle_color = match handle {
            EnvelopeHandle::Attack | EnvelopeHandle::AttackCurve => attack_color,
            EnvelopeHandle::DecaySustain => decay_sustain_color,
            EnvelopeHandle::DecayCurve => decay_color,
            EnvelopeHandle::Release | EnvelopeHandle::ReleaseCurve => release_color,
        };
        painter.circle_filled(
            position,
            radius + if highlighted { 1.0 } else { 0.0 },
            if is_curve {
                editor_theme::semantic().surface
            } else {
                handle_color
            },
        );
        painter.circle_stroke(
            position,
            radius + if highlighted { 1.0 } else { 0.0 },
            egui::Stroke::new(1.25_f32, handle_color),
        );
    }

    painter.text(
        egui::pos2(release_start.x + 5.0, top + 5.0),
        egui::Align2::LEFT_TOP,
        "NOTE OFF",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    let stage_legend = [
        ("A", P::Attack),
        ("D", P::Decay),
        ("S", P::Sustain),
        ("R", P::Release),
    ];
    for (index, (label, param)) in stage_legend.into_iter().enumerate() {
        let x = graph.left() + (index as f32 + 0.5) * graph.width() / 4.0;
        painter.text(
            egui::pos2(x, bottom + 2.0),
            egui::Align2::CENTER_TOP,
            format!("{label}  {}", state.format_param(param)),
            editor_theme::font::label(),
            editor_theme::semantic().text,
        );
    }

    editor_modulation::destination(
        ui,
        state,
        P::Attack,
        &attack_zone,
        attack,
        attack_zone.rect,
        editor_modulation::TrackAxis::Horizontal,
    );
    editor_modulation::destination_xy(
        ui,
        state,
        P::Decay,
        P::Sustain,
        &decay_zone,
        decay_zone.rect,
    );
    editor_modulation::destination(
        ui,
        state,
        P::Release,
        &release_zone,
        release,
        release_zone.rect,
        editor_modulation::TrackAxis::Horizontal,
    );
    editor_modulation::destination_xy(
        ui,
        state,
        P::AttackCurveTime,
        P::AttackCurve,
        &attack_curve_zone,
        attack_curve_zone.rect,
    );
    editor_modulation::destination_xy(
        ui,
        state,
        P::DecayCurveTime,
        P::DecayCurve,
        &decay_curve_zone,
        decay_curve_zone.rect,
    );
    editor_modulation::destination_xy(
        ui,
        state,
        P::ReleaseCurveTime,
        P::ReleaseCurve,
        &release_curve_zone,
        release_curve_zone.rect,
    );
}

fn destination_zone(ui: &egui::Ui, id: egui::Id, point: egui::Pos2) -> egui::Response {
    ui.interact(
        egui::Rect::from_center_size(point, egui::vec2(22.0, 22.0)),
        id,
        egui::Sense::hover(),
    )
}

fn envelope_horizon(seconds: f32) -> f32 {
    (seconds.max(0.0) * 1.15).max(0.5)
}

/// Shared two-axis curve-handle geometry for envelope and Shape editors.
pub(crate) fn curve_handle_position(
    start: egui::Pos2,
    end: egui::Pos2,
    handle_x: f32,
    handle_y: f32,
) -> egui::Pos2 {
    egui::pos2(
        egui::lerp(start.x..=end.x, handle_x.clamp(0.0, 1.0)),
        egui::lerp(start.y..=end.y, handle_y.clamp(0.0, 1.0)),
    )
}

fn envelope_curve_point(
    start: egui::Pos2,
    end: egui::Pos2,
    t: f32,
    handle_x: f32,
    handle_y: f32,
) -> egui::Pos2 {
    let warped_time = schlick_bias(t, 1.0 - handle_x.clamp(0.0, 1.0));
    let shaped = schlick_bias(warped_time, handle_y.clamp(0.0, 1.0));
    egui::pos2(
        egui::lerp(start.x..=end.x, t),
        egui::lerp(start.y..=end.y, shaped),
    )
}

fn schlick_bias(value: f32, bias: f32) -> f32 {
    let bias = bias.clamp(0.005, 0.995);
    value / ((bias.recip() - 2.0).mul_add(1.0 - value, 1.0))
}

fn push_envelope_curve(
    points: &mut Vec<egui::Pos2>,
    start: egui::Pos2,
    end: egui::Pos2,
    handle_x: f32,
    handle_y: f32,
) {
    for step in 0..=CURVE_POINTS {
        points.push(envelope_curve_point(
            start,
            end,
            f32::from(step) / f32::from(CURVE_POINTS),
            handle_x,
            handle_y,
        ));
    }
}
