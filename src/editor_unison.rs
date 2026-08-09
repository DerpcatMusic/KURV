//! Unison distribution, stereo blend, and direct point-shaper views.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::pan_curve::{
    PanShapeCurveData, PanShapeCurveState, PanShapeKnot, insert_knot, move_center, move_endpoint,
    move_knot, remove_knot, set_segment_curve,
};
use crate::voices::PanShapeSettings;
use crate::voices::{
    JITTER_EXCURSION_CENTS, MAX_UNISON, SwarmMode, UnisonAlignmentMode, UnisonSettings,
    fill_extended_unison_jitter_offsets, fill_extended_unison_layout,
    fill_unison_jitter_offsets_mode, stereo_pattern_center_seeded,
    unison_lane_position_stereo_jitter_seeded, unison_static_pitch_cents,
};
use crate::{
    KurvParams, P, editor_envelope, editor_modulation, editor_theme, editor_widgets,
    pan_shape_settings,
};

const CURVE_POINTS: u16 = 96;

#[derive(Clone, Copy)]
pub(crate) struct UnisonUiParams {
    pub(crate) voices: P,
    pub(crate) detune: P,
    pub(crate) harmonic_align: P,
    pub(crate) alignment_mode: P,
    pub(crate) detune_amount: P,
    pub(crate) stereo: P,
    pub(crate) phase: P,
    pub(crate) curve: P,
    pub(crate) jitter: P,
    pub(crate) jitter_rate: P,
    pub(crate) jitter_mode: P,
    pub(crate) stereo_alternate: P,
    pub(crate) stereo_x: P,
    pub(crate) weight: P,
    pub(crate) pan_center: P,
    pub(crate) pan_left: P,
    pub(crate) pan_right: P,
    pub(crate) pan_left_curve: P,
    pub(crate) pan_right_curve: P,
    pub(crate) pan_left_curve_time: P,
    pub(crate) pan_right_curve_time: P,
    pub(crate) pan_center_x: P,
    oscillator: u8,
}

impl UnisonUiParams {
    pub(crate) const OSC1: Self = Self {
        voices: P::UnisonVoices,
        detune: P::UnisonDetune,
        harmonic_align: P::UnisonHarmonicAlign,
        alignment_mode: P::UnisonAlignmentMode,
        detune_amount: P::UnisonDetuneAmount,
        stereo: P::UnisonStereo,
        phase: P::PhaseRandom,
        curve: P::UnisonCurve,
        jitter: P::UnisonSwarm,
        jitter_rate: P::UnisonSwarmRate,
        jitter_mode: P::UnisonSwarmMode,
        stereo_alternate: P::StereoAlternate,
        stereo_x: P::StereoX,
        weight: P::UnisonWeight,
        pan_center: P::PanShapeCenter,
        pan_left: P::PanShapeLeft,
        pan_right: P::PanShapeRight,
        pan_left_curve: P::PanShapeLeftCurve,
        pan_right_curve: P::PanShapeRightCurve,
        pan_left_curve_time: P::PanShapeLeftCurveTime,
        pan_right_curve_time: P::PanShapeRightCurveTime,
        pan_center_x: P::PanShapeCenterX,
        oscillator: 0,
    };

    pub(crate) const OSC2: Self = Self {
        voices: P::Osc2UnisonVoices,
        detune: P::Osc2UnisonDetune,
        harmonic_align: P::Osc2UnisonHarmonicAlign,
        alignment_mode: P::Osc2UnisonAlignmentMode,
        detune_amount: P::Osc2UnisonDetuneAmount,
        stereo: P::Osc2UnisonStereo,
        phase: P::Osc2PhaseRandom,
        curve: P::Osc2UnisonCurve,
        jitter: P::Osc2UnisonJitter,
        jitter_rate: P::Osc2UnisonJitterRate,
        jitter_mode: P::Osc2JitterMode,
        stereo_alternate: P::Osc2StereoAlternate,
        stereo_x: P::Osc2StereoX,
        weight: P::Osc2UnisonWeight,
        pan_center: P::Osc2PanShapeCenter,
        pan_left: P::Osc2PanShapeLeft,
        pan_right: P::Osc2PanShapeRight,
        pan_left_curve: P::Osc2PanShapeLeftCurve,
        pan_right_curve: P::Osc2PanShapeRightCurve,
        pan_left_curve_time: P::Osc2PanShapeLeftCurveTime,
        pan_right_curve_time: P::Osc2PanShapeRightCurveTime,
        pan_center_x: P::Osc2PanShapeCenterX,
        oscillator: 1,
    };

    pub(crate) const OSC3: Self = Self {
        voices: P::Osc3UnisonVoices,
        detune: P::Osc3UnisonDetune,
        harmonic_align: P::Osc3UnisonHarmonicAlign,
        alignment_mode: P::Osc3UnisonAlignmentMode,
        detune_amount: P::Osc3UnisonDetuneAmount,
        stereo: P::Osc3UnisonStereo,
        phase: P::Osc3PhaseRandom,
        curve: P::Osc3UnisonCurve,
        jitter: P::Osc3UnisonJitter,
        jitter_rate: P::Osc3UnisonJitterRate,
        jitter_mode: P::Osc3JitterMode,
        stereo_alternate: P::Osc3StereoAlternate,
        stereo_x: P::Osc3StereoX,
        weight: P::Osc3UnisonWeight,
        pan_center: P::Osc3PanShapeCenter,
        pan_left: P::Osc3PanShapeLeft,
        pan_right: P::Osc3PanShapeRight,
        pan_left_curve: P::Osc3PanShapeLeftCurve,
        pan_right_curve: P::Osc3PanShapeRightCurve,
        pan_left_curve_time: P::Osc3PanShapeLeftCurveTime,
        pan_right_curve_time: P::Osc3PanShapeRightCurveTime,
        pan_center_x: P::Osc3PanShapeCenterX,
        oscillator: 2,
    };
}

fn plain_param_value(state: &PluginContext<KurvParams>, id: P) -> f32 {
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == u32::from(id))
        .map_or_else(
            || state.get_param(id),
            |info| info.range.denormalize(f64::from(state.get_param(id))) as f32,
        )
}

fn traced_begin(state: &PluginContext<KurvParams>, control: &'static str, id: P, x: f32, y: f32) {
    crate::diagnostics::trace(control, "begin-enter", x, y);
    state.begin_edit(id);
    crate::diagnostics::trace(control, "begin-return", x, y);
}

fn traced_set(
    state: &PluginContext<KurvParams>,
    control: &'static str,
    axis: &'static str,
    id: P,
    value: f32,
    x: f32,
    y: f32,
) {
    crate::diagnostics::trace(control, axis, x, y);
    state.set_param(id, f64::from(value));
    crate::diagnostics::trace(
        control,
        match axis {
            "set-x-enter" => "set-x-return",
            _ => "set-y-return",
        },
        x,
        y,
    );
}

fn traced_end(state: &PluginContext<KurvParams>, control: &'static str, id: P, x: f32, y: f32) {
    crate::diagnostics::trace(control, "end-enter", x, y);
    state.end_edit(id);
    crate::diagnostics::trace(control, "end-return", x, y);
}

fn pan_shape_curve_state(params: &KurvParams, binding: UnisonUiParams) -> &PanShapeCurveState {
    match binding.oscillator {
        0 => &params.pan_shape_curve_state,
        1 => &params.osc2_pan_shape_curve_state,
        _ => &params.osc3_pan_shape_curve_state,
    }
}

fn preview_meters(state: &PluginContext<KurvParams>, binding: UnisonUiParams) -> (f32, f32) {
    let params = state.params();
    match binding.oscillator {
        0 => (
            state.get_meter(&params.stereo_seed),
            state.get_meter(&params.swarm_phase),
        ),
        1 => (
            state.get_meter(&params.osc2_stereo_seed),
            state.get_meter(&params.osc2_swarm_phase),
        ),
        _ => (
            state.get_meter(&params.osc3_stereo_seed),
            state.get_meter(&params.osc3_swarm_phase),
        ),
    }
}

fn pan_shape_settings_for(
    state: &PluginContext<KurvParams>,
    binding: UnisonUiParams,
) -> PanShapeSettings {
    if binding.oscillator == 0 {
        let mut settings = pan_shape_settings(state.params());
        let center = editor_modulation::effective_plain_value(state, binding.pan_center);
        let center_x = editor_modulation::effective_plain_value(state, binding.pan_center_x);
        let left = editor_modulation::effective_plain_value(state, binding.pan_left);
        let right = editor_modulation::effective_plain_value(state, binding.pan_right);
        settings.center = center.clamp(0.0, 1.0);
        settings.center_x = center_x.clamp(0.05, 0.95);
        settings.left_edge = left.clamp(0.0, 1.0);
        settings.right_edge = right.clamp(0.0, 1.0);
        settings.left_curve =
            editor_modulation::effective_plain_value(state, binding.pan_left_curve)
                .clamp(-1.0, 1.0);
        settings.right_curve =
            editor_modulation::effective_plain_value(state, binding.pan_right_curve)
                .clamp(-1.0, 1.0);
        settings.left_curve_time =
            editor_modulation::effective_plain_value(state, binding.pan_left_curve_time)
                .clamp(0.05, 0.95);
        settings.right_curve_time =
            editor_modulation::effective_plain_value(state, binding.pan_right_curve_time)
                .clamp(0.05, 0.95);
        if settings.left_segments.count > 0 {
            settings.left_segments.seg_p0[0] = settings.center;
            settings.left_segments.seg_p3[usize::from(settings.left_segments.count - 1)] =
                settings.left_edge;
        }
        if settings.right_segments.count > 0 {
            settings.right_segments.seg_p0[0] = settings.center;
            settings.right_segments.seg_p3[usize::from(settings.right_segments.count - 1)] =
                settings.right_edge;
        }
        return settings;
    }
    let center = editor_modulation::effective_plain_value(state, binding.pan_center);
    let center_x = editor_modulation::effective_plain_value(state, binding.pan_center_x);
    let left = editor_modulation::effective_plain_value(state, binding.pan_left);
    let right = editor_modulation::effective_plain_value(state, binding.pan_right);
    let left_curve = editor_modulation::effective_plain_value(state, binding.pan_left_curve);
    let right_curve = editor_modulation::effective_plain_value(state, binding.pan_right_curve);
    let left_time = editor_modulation::effective_plain_value(state, binding.pan_left_curve_time);
    let right_time = editor_modulation::effective_plain_value(state, binding.pan_right_curve_time);
    let curve_state = pan_shape_curve_state(state.params(), binding);
    let data = if curve_state.is_initialized() {
        curve_state.snapshot()
    } else {
        PanShapeCurveData::from_legacy(
            center,
            left,
            right,
            left_curve,
            right_curve,
            left_time,
            right_time,
        )
    };
    PanShapeSettings::new(center, 1.0, 0.0)
        .with_center_x(center_x)
        .with_sides(left, right, left_curve, right_curve)
        .with_curve_times(left_time, right_time)
        .with_curve_data(&data)
}

#[derive(Clone)]
struct PanShapePointDrag {
    target: PanShapePointDragTarget,
    anchor: egui::Pos2,
}

#[derive(Clone, Copy)]
enum PanShapePointDragTarget {
    Center,
    Endpoint { left: bool },
    Knot { left: bool, index: usize },
    Curve { left: bool, index: usize },
}

fn constrain_drag(anchor: egui::Pos2, pointer: egui::Pos2, enabled: bool) -> egui::Pos2 {
    if !enabled {
        return pointer;
    }
    let delta = pointer - anchor;
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    [
        (1.0, 0.0),
        (0.0, 1.0),
        (diagonal, diagonal),
        (diagonal, -diagonal),
    ]
    .into_iter()
    .map(|(x, y)| {
        let direction = egui::vec2(x, y);
        let projected = anchor + direction * delta.dot(direction);
        (projected.distance_sq(pointer), projected)
    })
    .min_by(|left, right| left.0.total_cmp(&right.0))
    .map_or(pointer, |(_, projected)| projected)
}

pub(crate) fn unison_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    params: UnisonUiParams,
    interactive: bool,
    clear_background: bool,
) {
    let voices = plain_param_value(state, params.voices)
        .round()
        .clamp(1.0, 64.0) as u8;
    let detune_range =
        editor_modulation::effective_plain_value(state, params.detune).clamp(0.0, 48.0);
    let harmonic_align =
        editor_modulation::effective_plain_value(state, params.harmonic_align).clamp(0.0, 1.0);
    let harmonic_align_normalized = state.get_param(params.harmonic_align).clamp(0.0, 1.0);
    let alignment_mode = UnisonAlignmentMode::from_index(
        plain_param_value(state, params.alignment_mode)
            .round()
            .clamp(0.0, 3.0) as u8,
    );
    let detune_amount =
        editor_modulation::effective_plain_value(state, params.detune_amount).clamp(0.0, 1.0);
    let detune_amount_normalized = state.get_param(params.detune_amount);
    let curve = editor_modulation::effective_plain_value(state, params.curve).clamp(-1.0, 1.0);
    let curve_normalized = state.get_param(params.curve);
    let alternate =
        editor_modulation::effective_plain_value(state, params.stereo_alternate).clamp(0.0, 1.0);
    let stereo_x = editor_modulation::effective_plain_value(state, params.stereo_x).clamp(0.0, 1.0);
    let level_curve =
        editor_modulation::effective_plain_value(state, params.weight).clamp(-1.0, 1.0);
    let level_normalized = state.get_param(params.weight);
    let pan_shape = pan_shape_settings_for(state, params);
    let stereo = editor_modulation::effective_plain_value(state, params.stereo).clamp(0.0, 1.0);
    let (random_seed, swarm_time) = preview_meters(state, params);
    let random_seed = random_seed.clamp(0.0, 1.0);
    let swarm_amount =
        editor_modulation::effective_plain_value(state, params.jitter).clamp(0.0, 1.0);
    let swarm_mode = SwarmMode::from_index(
        plain_param_value(state, params.jitter_mode)
            .round()
            .clamp(0.0, 1.0) as u8,
    );
    let swarm_time = swarm_time.max(0.0);
    if swarm_amount > f32::EPSILON {
        editor_theme::request_display_repaint(ui);
    }
    let (outer, painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
    let rect = outer.rect;
    let inner = editor_widgets::graph_plot(
        rect,
        ui,
        editor_theme::title_height(ui),
        editor_theme::space::XS,
    );
    let slider_height = editor_theme::metrics(ui).points(3.5).clamp(48.0, 56.0);
    let plot = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(inner.right(), inner.bottom() - slider_height),
    );
    let slider = egui::Rect::from_min_max(egui::pos2(inner.left(), plot.bottom()), inner.max);
    let weight_slider =
        egui::Rect::from_min_max(slider.min, egui::pos2(slider.right(), slider.center().y));
    let harmonic_row =
        egui::Rect::from_min_max(egui::pos2(slider.left(), slider.center().y), slider.max);
    let alignment_mode_width = harmonic_row.width().min(60.0);
    let alignment_mode_gap = 4.0_f32.min(harmonic_row.width() * 0.08);
    let harmonic_slider = egui::Rect::from_min_size(
        harmonic_row.min,
        egui::vec2(
            (harmonic_row.width() - alignment_mode_width - alignment_mode_gap).max(1.0),
            harmonic_row.height(),
        ),
    );
    let alignment_mode_slider = egui::Rect::from_min_max(
        egui::pos2(
            harmonic_slider.right() + alignment_mode_gap,
            harmonic_row.top(),
        ),
        harmonic_row.max,
    );
    let plot_response = ui
        .interact(
            plot,
            outer.id.with("distribution"),
            if interactive {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            },
        )
        .on_hover_cursor(egui::CursorIcon::Crosshair);
    let weight_response = ui
        .interact(
            weight_slider,
            outer.id.with("weight"),
            if interactive {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            },
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let harmonic_response = ui
        .interact(
            harmonic_slider,
            outer.id.with("harmonic-align"),
            if interactive {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::hover()
            },
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text(
            "Moves each static unison pitch toward its NOTE or harmonic partial target; JITTER remains independent",
        );
    let alignment_mode_response = ui
        .interact(
            alignment_mode_slider,
            outer.id.with("alignment-mode"),
            if interactive {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        )
        .on_hover_text("NOTE, HARM, ODD, or EVEN harmonic partial targets");
    let modulation_gesture = interactive
        && (editor_modulation::owns_gesture(ui, state, params.detune_amount, &plot_response)
            || editor_modulation::owns_gesture(ui, state, params.curve, &plot_response));

    // One CLAP gesture owns the two-axis edit. Both values are still sent as
    // automatable parameter changes, but Bitwig never sees nested gestures.
    if interactive && !modulation_gesture && plot_response.drag_started() {
        traced_begin(
            state,
            "unison-distribution",
            params.detune_amount,
            detune_amount_normalized,
            curve_normalized,
        );
    }
    if !modulation_gesture
        && (plot_response.drag_started() || plot_response.dragged())
        && let Some(position) = plot_response.interact_pointer_pos()
    {
        let detune_norm = ((position.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        let curve_norm = (1.0 - (position.y - plot.top()) / plot.height()).clamp(0.0, 1.0);
        traced_set(
            state,
            "unison-distribution",
            "set-x-enter",
            params.detune_amount,
            detune_norm,
            detune_norm,
            curve_norm,
        );
        traced_set(
            state,
            "unison-distribution",
            "set-y-enter",
            params.curve,
            curve_norm,
            detune_norm,
            curve_norm,
        );
    }
    if !modulation_gesture && plot_response.drag_stopped() {
        traced_end(
            state,
            "unison-distribution",
            params.detune_amount,
            state.get_param(params.detune_amount),
            state.get_param(params.curve),
        );
    }
    if interactive && weight_response.drag_started() {
        state.begin_edit(params.weight);
    }
    if (weight_response.drag_started() || weight_response.dragged())
        && let Some(position) = weight_response.interact_pointer_pos()
    {
        let normalized =
            ((position.x - weight_slider.left()) / weight_slider.width()).clamp(0.0, 1.0);
        if (state.get_param(params.weight) - normalized).abs() > f32::EPSILON {
            state.set_param(params.weight, f64::from(normalized));
        }
    }
    if weight_response.drag_stopped() {
        state.end_edit(params.weight);
    }
    if interactive
        && weight_response.clicked()
        && let Some(position) = weight_response.interact_pointer_pos()
    {
        let normalized =
            ((position.x - weight_slider.left()) / weight_slider.width()).clamp(0.0, 1.0);
        state.begin_edit(params.weight);
        state.set_param(params.weight, f64::from(normalized));
        state.end_edit(params.weight);
    }

    if interactive && harmonic_response.drag_started() {
        state.begin_edit(params.harmonic_align);
    }
    if (harmonic_response.drag_started() || harmonic_response.dragged())
        && let Some(position) = harmonic_response.interact_pointer_pos()
    {
        let normalized =
            ((position.x - harmonic_slider.left()) / harmonic_slider.width()).clamp(0.0, 1.0);
        if (state.get_param(params.harmonic_align) - normalized).abs() > f32::EPSILON {
            state.set_param(params.harmonic_align, f64::from(normalized));
        }
    }
    if harmonic_response.drag_stopped() {
        state.end_edit(params.harmonic_align);
    }
    if interactive
        && harmonic_response.clicked()
        && let Some(position) = harmonic_response.interact_pointer_pos()
    {
        let normalized =
            ((position.x - harmonic_slider.left()) / harmonic_slider.width()).clamp(0.0, 1.0);
        state.begin_edit(params.harmonic_align);
        state.set_param(params.harmonic_align, f64::from(normalized));
        state.end_edit(params.harmonic_align);
    }
    if interactive && alignment_mode_response.clicked() {
        let next_mode = UnisonAlignmentMode::from_index((alignment_mode.index() + 1) % 4);
        state.automate(params.alignment_mode, f64::from(next_mode.index()) / 3.0);
    }

    if clear_background {
        editor_widgets::graph_frame(&painter, rect);
    }
    if interactive {
        editor_widgets::graph_title(&painter, rect, "UNISON DISTRIBUTION");
    }
    editor_widgets::graph_grid(&painter, plot, 4, 4);
    let grid = egui::Stroke::new(1.0_f32, editor_theme::semantic().grid);
    painter.line_segment(
        [
            egui::pos2(plot.left(), plot.center().y),
            egui::pos2(plot.right(), plot.center().y),
        ],
        grid,
    );
    painter.line_segment(
        [
            egui::pos2(plot.center().x, plot.top()),
            egui::pos2(plot.center().x, plot.bottom()),
        ],
        grid,
    );
    painter.line_segment([weight_slider.left_top(), weight_slider.right_top()], grid);
    painter.line_segment(
        [harmonic_slider.left_top(), harmonic_slider.right_top()],
        grid,
    );

    let (pan_center, pan_scale) = stereo_pattern_center_seeded(
        voices,
        curve,
        alternate,
        stereo_x,
        level_curve,
        pan_shape,
        random_seed,
    );
    let mut jitter_offsets = [0.0; MAX_UNISON];
    fill_unison_jitter_offsets_mode(
        &mut jitter_offsets[..usize::from(voices)],
        random_seed,
        swarm_amount,
        swarm_time,
        swarm_mode,
    );
    let mut maximum_weight = f32::EPSILON;
    for (index, jitter) in jitter_offsets[..usize::from(voices)].iter().enumerate() {
        let (_, _, weight) = unison_lane_position_stereo_jitter_seeded(
            voices,
            index,
            curve,
            alternate,
            stereo_x,
            level_curve,
            pan_shape,
            random_seed,
            detune_amount,
            harmonic_align,
            alignment_mode,
            *jitter,
            detune_range * 100.0,
        );
        maximum_weight = maximum_weight.max(weight);
    }
    // Keep the detune axis stable while the amount is edited. The scale
    // represents the full Range, so a 1% amount renders at 1% width instead
    // of being fit back to the graph edges. Include bounded JITTER so
    // animated lanes cannot clip.
    let detune_full_scale =
        (detune_range * 100.0 + JITTER_EXCURSION_CENTS * swarm_amount.clamp(0.0, 1.0)).max(1.0);
    let detune_width = plot.width() * 0.5 / detune_full_scale;
    for index in 0..voices {
        let (detune_position, pan_position, weight) = unison_lane_position_stereo_jitter_seeded(
            voices,
            usize::from(index),
            curve,
            alternate,
            stereo_x,
            level_curve,
            pan_shape,
            random_seed,
            detune_amount,
            harmonic_align,
            alignment_mode,
            jitter_offsets[usize::from(index)],
            detune_range * 100.0,
        );
        let pan = ((pan_position - pan_center) * pan_scale * stereo).clamp(-1.0, 1.0);
        let center = egui::pos2(
            detune_position.mul_add(detune_width, plot.center().x),
            (-pan).mul_add(plot.height() * 0.38, plot.center().y),
        );
        let relative_weight = (weight / maximum_weight).sqrt();
        let line_height = relative_weight.mul_add(14.0, 5.0);
        let color = editor_theme::semantic()
            .unison
            .linear_multiply(relative_weight.mul_add(0.72, 0.28));
        painter.line_segment(
            [
                center - egui::vec2(0.0, line_height),
                center + egui::vec2(0.0, line_height),
            ],
            egui::Stroke::new(2.0_f32, color),
        );
    }

    let control_bounds = plot.shrink(4.0);
    let control_point = egui::pos2(
        control_bounds
            .width()
            .mul_add(detune_amount_normalized, control_bounds.left()),
        control_bounds
            .height()
            .mul_add(-curve_normalized, control_bounds.bottom()),
    );
    painter.circle_filled(control_point, 3.0, editor_theme::semantic().unison);

    painter.text(
        plot.right_bottom() + egui::vec2(-9.0, -7.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("±{:.2} st", detune_range * detune_amount),
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );

    let weight_track = egui::Rect::from_center_size(
        egui::pos2(weight_slider.center().x, weight_slider.top() + 9.0),
        egui::vec2((weight_slider.width() - 12.0).max(1.0), 3.0),
    );
    painter.rect_filled(weight_track, 1.5, editor_theme::semantic().grid);
    let weight_filled = egui::Rect::from_min_max(
        egui::pos2(
            egui::lerp(
                weight_track.left()..=weight_track.right(),
                level_normalized.min(0.5),
            ),
            weight_track.top(),
        ),
        egui::pos2(
            egui::lerp(
                weight_track.left()..=weight_track.right(),
                level_normalized.max(0.5),
            ),
            weight_track.bottom(),
        ),
    );
    painter.rect_filled(weight_filled, 1.5, editor_theme::semantic().unison);
    let weight_marker = egui::pos2(
        egui::lerp(weight_track.left()..=weight_track.right(), level_normalized),
        weight_track.center().y,
    );
    painter.circle_filled(weight_marker, 4.0, editor_theme::semantic().unison);
    painter.text(
        weight_slider.left_bottom() + egui::vec2(6.0, -2.0),
        egui::Align2::LEFT_BOTTOM,
        "CENTER",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        weight_slider.center_bottom() + egui::vec2(0.0, -2.0),
        egui::Align2::CENTER_BOTTOM,
        "EVEN",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        weight_slider.right_bottom() + egui::vec2(-6.0, -2.0),
        egui::Align2::RIGHT_BOTTOM,
        "SIDES",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );

    let harmonic_track = egui::Rect::from_center_size(
        egui::pos2(harmonic_slider.center().x, harmonic_slider.top() + 9.0),
        egui::vec2((harmonic_slider.width() - 12.0).max(1.0), 3.0),
    );
    painter.rect_filled(harmonic_track, 1.5, editor_theme::semantic().grid);
    let harmonic_filled = egui::Rect::from_min_max(
        harmonic_track.left_top(),
        egui::pos2(
            egui::lerp(
                harmonic_track.left()..=harmonic_track.right(),
                harmonic_align_normalized,
            ),
            harmonic_track.bottom(),
        ),
    );
    painter.rect_filled(harmonic_filled, 1.5, editor_theme::semantic().unison);
    let harmonic_marker = egui::pos2(
        egui::lerp(
            harmonic_track.left()..=harmonic_track.right(),
            harmonic_align_normalized,
        ),
        harmonic_track.center().y,
    );
    painter.circle_filled(harmonic_marker, 4.0, editor_theme::semantic().unison);
    painter.text(
        harmonic_slider.left_bottom() + egui::vec2(6.0, -2.0),
        egui::Align2::LEFT_BOTTOM,
        "FREE",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        harmonic_slider.center_bottom() + egui::vec2(0.0, -2.0),
        egui::Align2::CENTER_BOTTOM,
        format!("ALIGN {:.0}%", harmonic_align * 100.0),
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        harmonic_slider.right_bottom() + egui::vec2(-6.0, -2.0),
        egui::Align2::RIGHT_BOTTOM,
        "LOCK",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    painter.rect_filled(
        alignment_mode_slider,
        1.5,
        if alignment_mode_response.hovered() {
            editor_theme::semantic().control_hover
        } else {
            editor_theme::semantic().control
        },
    );
    painter.text(
        alignment_mode_slider.center_top() + egui::vec2(0.0, 2.0),
        egui::Align2::CENTER_TOP,
        "MODE",
        editor_theme::font::caption(),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        alignment_mode_slider.center_bottom() - egui::vec2(0.0, 2.0),
        egui::Align2::CENTER_BOTTOM,
        alignment_mode.label(),
        editor_theme::font::value(),
        ui.visuals().text_color(),
    );

    painter.text(
        plot.right_top() + egui::vec2(-7.0, 6.0),
        egui::Align2::RIGHT_TOP,
        "R",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    painter.text(
        plot.right_bottom() + egui::vec2(-7.0, -20.0),
        egui::Align2::RIGHT_BOTTOM,
        "L",
        editor_theme::font::label(),
        editor_theme::semantic().text_muted,
    );
    editor_modulation::destination_xy(
        ui,
        state,
        params.detune_amount,
        params.curve,
        &plot_response,
        plot,
    );
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CompactUnisonView {
    #[default]
    Unison,
    PanShape,
}

fn compact_unison_layout(rect: egui::Rect) -> (egui::Rect, egui::Rect, egui::Rect, egui::Rect) {
    let content = rect.shrink(3.0);
    let header_height = 13.0_f32.min(content.height() * 0.18);
    let header = egui::Rect::from_min_size(
        content.min,
        egui::vec2(content.width(), header_height.min(content.height())),
    );
    let view = egui::Rect::from_min_max(
        egui::pos2(
            content.left(),
            (header.bottom() - 1.0).min(content.bottom()),
        ),
        content.max,
    );
    let rail_width = (content.width() * 0.05).clamp(10.0, 14.0);
    let rail = egui::Rect::from_min_max(
        egui::pos2((view.right() - rail_width).max(view.left()), view.top()),
        view.max,
    );
    let plot = egui::Rect::from_min_max(
        view.min,
        egui::pos2((rail.left() - 2.0).max(view.left()), view.bottom()),
    );
    (header, view, plot, rail)
}

fn compact_pan_shape_panes(rect: egui::Rect) -> (egui::Rect, egui::Rect) {
    let divider = rect.center().x;
    (
        egui::Rect::from_min_max(rect.min, egui::pos2(divider - 1.0, rect.bottom())),
        egui::Rect::from_min_max(egui::pos2(divider + 1.0, rect.top()), rect.max),
    )
}

fn paint_compact_pan_shape_divider(painter: &egui::Painter, rect: egui::Rect) {
    painter.line_segment(
        [
            egui::pos2(rect.center().x, rect.top() + 3.0),
            egui::pos2(rect.center().x, rect.bottom() - 3.0),
        ],
        egui::Stroke::new(
            1.0_f32,
            editor_theme::semantic().pan_shape.gamma_multiply(0.24),
        ),
    );
}

fn compact_unison_view_tabs(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    header: egui::Rect,
    id: egui::Id,
    current: CompactUnisonView,
) -> CompactUnisonView {
    let palette = editor_theme::semantic();
    let mut selected = current;
    let tabs = [
        (CompactUnisonView::Unison, "UNISON", 44.0_f32),
        (CompactUnisonView::PanShape, "PAN SHAPE", 58.0_f32),
    ];
    let mut left = header.left();
    for (view, label, width) in tabs {
        let rect = egui::Rect::from_min_size(
            egui::pos2(left, header.top()),
            egui::vec2(width.min((header.right() - left).max(0.0)), header.height()),
        );
        let response = ui
            .interact(rect, id.with(label), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            selected = view;
        }
        let active = view == selected;
        let accent = if matches!(view, CompactUnisonView::Unison) {
            palette.unison
        } else {
            palette.pan_shape
        };
        if active {
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                egui::Stroke::new(1.25_f32, accent),
            );
        }
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            editor_theme::font::caption(),
            accent,
        );
        left = rect.right() + 1.0;
    }
    if selected != current {
        ui.data_mut(|data| data.insert_temp(id, selected));
    }
    selected
}

fn compact_alignment_mode_combo(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    current: UnisonAlignmentMode,
) -> Option<UnisonAlignmentMode> {
    let mut selected = None;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect());
    child.spacing_mut().interact_size.y = rect.height();
    child.spacing_mut().button_padding = egui::vec2(4.0, 1.0);
    let palette = editor_theme::semantic();
    let visuals = child.visuals_mut();
    visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.bg_fill = plugcat::theme::mix(palette.well, palette.unison, 0.12);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.bg_fill = plugcat::theme::mix(palette.well, palette.unison, 0.20);
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    egui::ComboBox::from_id_salt(id.with("menu"))
        .selected_text(
            egui::RichText::new(current.label())
                .font(editor_theme::font::value())
                .color(palette.unison),
        )
        .width(rect.width())
        .show_ui(&mut child, |ui| {
            for mode in [
                UnisonAlignmentMode::Note,
                UnisonAlignmentMode::Harmonic,
                UnisonAlignmentMode::Odd,
                UnisonAlignmentMode::Even,
            ] {
                if ui
                    .selectable_label(
                        mode == current,
                        egui::RichText::new(mode.label())
                            .font(editor_theme::font::label())
                            .color(palette.unison),
                    )
                    .clicked()
                {
                    selected = Some(mode);
                }
            }
        });
    selected
}

fn paint_compact_alignment_rail(
    painter: &egui::Painter,
    rect: egui::Rect,
    value: f32,
    hovered: bool,
) {
    let palette = editor_theme::semantic();
    painter.rect_filled(
        rect,
        1.5,
        plugcat::theme::mix(
            palette.well,
            palette.unison,
            if hovered { 0.20 } else { 0.10 },
        ),
    );
    let track = rect.shrink2(egui::vec2(rect.width() * 0.42, 3.0));
    painter.line_segment(
        [track.center_bottom(), track.center_top()],
        egui::Stroke::new(1.0_f32, palette.unison.gamma_multiply(0.45)),
    );
    let y = egui::lerp(track.bottom()..=track.top(), value.clamp(0.0, 1.0));
    painter.line_segment(
        [track.center_bottom(), egui::pos2(track.center().x, y)],
        egui::Stroke::new(2.0_f32, palette.unison),
    );
    painter.circle_filled(egui::pos2(track.center().x, y), 3.25, palette.unison);
}

fn paint_compact_pan_shape(
    painter: &egui::Painter,
    rect: egui::Rect,
    center_x: f32,
    center: f32,
    label: &str,
    value_at: impl Fn(bool, f32) -> f32,
) {
    let palette = editor_theme::semantic();
    let plot = rect.shrink2(egui::vec2(5.0, 3.0));
    let center_x = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    for left in [true, false] {
        let points = (0..=32)
            .map(|index| {
                let input = index as f32 / 32.0;
                let x = if left {
                    egui::lerp(center_x..=plot.left(), input)
                } else {
                    egui::lerp(center_x..=plot.right(), input)
                };
                egui::pos2(
                    x,
                    egui::lerp(
                        plot.bottom()..=plot.top(),
                        value_at(left, input).clamp(0.0, 1.0),
                    ),
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.35_f32, palette.pan_shape),
        ));
    }
    painter.circle_filled(
        egui::pos2(
            center_x,
            egui::lerp(plot.bottom()..=plot.top(), center.clamp(0.0, 1.0)),
        ),
        2.75,
        palette.pan_shape,
    );
    painter.text(
        plot.left_top(),
        egui::Align2::LEFT_TOP,
        label,
        editor_theme::font::caption(),
        palette.pan_shape,
    );
}

/// The in-oscillator unison view switches the full plot between distribution
/// and pan-shape editing while keeping the choice local to this editor.
pub(crate) fn compact_unison_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    params: UnisonUiParams,
) {
    let voices = plain_param_value(state, params.voices)
        .round()
        .clamp(1.0, 64.0) as u8;
    let detune_range =
        editor_modulation::effective_plain_value(state, params.detune).clamp(0.0, 48.0);
    let detune_amount =
        editor_modulation::effective_plain_value(state, params.detune_amount).clamp(0.0, 1.0);
    let detune_amount_normalized = state.get_param(params.detune_amount).clamp(0.0, 1.0);
    let curve = editor_modulation::effective_plain_value(state, params.curve).clamp(-1.0, 1.0);
    let curve_normalized = state.get_param(params.curve).clamp(0.0, 1.0);
    let harmonic_align =
        editor_modulation::effective_plain_value(state, params.harmonic_align).clamp(0.0, 1.0);
    let alignment_mode = UnisonAlignmentMode::from_index(
        plain_param_value(state, params.alignment_mode)
            .round()
            .clamp(0.0, 3.0) as u8,
    );
    let alternate =
        editor_modulation::effective_plain_value(state, params.stereo_alternate).clamp(0.0, 1.0);
    let stereo_x = editor_modulation::effective_plain_value(state, params.stereo_x).clamp(0.0, 1.0);
    let level_curve =
        editor_modulation::effective_plain_value(state, params.weight).clamp(-1.0, 1.0);
    let pan_shape = pan_shape_settings_for(state, params);
    let stereo = editor_modulation::effective_plain_value(state, params.stereo).clamp(0.0, 1.0);
    let swarm_amount =
        editor_modulation::effective_plain_value(state, params.jitter).clamp(0.0, 1.0);
    let swarm_mode = SwarmMode::from_index(
        plain_param_value(state, params.jitter_mode)
            .round()
            .clamp(0.0, 1.0) as u8,
    );
    let (random_seed, swarm_time) = preview_meters(state, params);
    if swarm_amount > f32::EPSILON {
        editor_theme::request_display_repaint(ui);
    }

    let (outer, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::hover(),
    );
    let (header, view_rect, unison_plot, alignment_rail) = compact_unison_layout(outer.rect);
    painter.rect_filled(outer.rect, 0.0, editor_theme::semantic().well);
    let view_id = outer.id.with("view");
    let current_view = ui
        .data(|data| data.get_temp::<CompactUnisonView>(view_id))
        .unwrap_or_default();
    let selected_view = compact_unison_view_tabs(ui, &painter, header, view_id, current_view);
    if matches!(selected_view, CompactUnisonView::Unison) {
        let mode_width = (header.width() * 0.30).clamp(56.0, 70.0);
        let mode_rect = egui::Rect::from_min_max(
            egui::pos2(
                (header.right() - mode_width).max(header.left()),
                header.top(),
            ),
            header.max,
        );
        painter.text(
            egui::pos2(
                (header.left() + 118.0).min(mode_rect.left()),
                header.center().y,
            ),
            egui::Align2::LEFT_CENTER,
            format!("ALIGN {:.0}%", harmonic_align * 100.0),
            editor_theme::font::caption(),
            editor_theme::semantic().unison,
        );
        if let Some(mode) = compact_alignment_mode_combo(
            ui,
            mode_rect,
            outer.id.with("alignment-mode"),
            alignment_mode,
        ) && mode != alignment_mode
        {
            state.begin_edit(params.alignment_mode);
            state.set_param(params.alignment_mode, f64::from(mode.index()) / 3.0);
            state.end_edit(params.alignment_mode);
        }
    }

    let mut jitter_offsets = [0.0; MAX_UNISON];
    fill_unison_jitter_offsets_mode(
        &mut jitter_offsets[..usize::from(voices)],
        random_seed.clamp(0.0, 1.0),
        swarm_amount,
        swarm_time.max(0.0),
        swarm_mode,
    );
    let (pan_center, pan_scale) = stereo_pattern_center_seeded(
        voices,
        curve,
        alternate,
        stereo_x,
        level_curve,
        pan_shape,
        random_seed.clamp(0.0, 1.0),
    );
    let full_scale = (detune_range * 100.0 + JITTER_EXCURSION_CENTS * swarm_amount).max(1.0);
    let distribution_plot = unison_plot;
    let mut points = [egui::Pos2::ZERO; MAX_UNISON];
    let mut weights = [1.0_f32; MAX_UNISON];
    let mut maximum_weight = f32::EPSILON;
    for index in 0..usize::from(voices) {
        let (detune, pan, weight) = unison_lane_position_stereo_jitter_seeded(
            voices,
            index,
            curve,
            alternate,
            stereo_x,
            level_curve,
            pan_shape,
            random_seed.clamp(0.0, 1.0),
            detune_amount,
            harmonic_align,
            alignment_mode,
            jitter_offsets[index],
            detune_range * 100.0,
        );
        let pan = ((pan - pan_center) * pan_scale * stereo).clamp(-1.0, 1.0);
        points[index] = egui::pos2(
            (detune / full_scale).mul_add(
                distribution_plot.width() * 0.46,
                distribution_plot.center().x,
            ),
            (-pan).mul_add(
                distribution_plot.height() * 0.38,
                distribution_plot.center().y,
            ),
        );
        weights[index] = weight;
        maximum_weight = maximum_weight.max(weight);
    }
    match selected_view {
        CompactUnisonView::Unison => {
            let response = ui
                .interact(
                    unison_plot,
                    outer.id.with("distribution"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Crosshair)
                .on_hover_text("Drag X for detune amount; drag Y for distribution curve");
            let modulation_gesture =
                editor_modulation::owns_gesture(ui, state, params.detune_amount, &response)
                    || editor_modulation::owns_gesture(ui, state, params.curve, &response);
            if !modulation_gesture && response.drag_started() {
                for id in [params.detune_amount, params.curve] {
                    traced_begin(
                        state,
                        "compact-unison",
                        id,
                        detune_amount_normalized,
                        curve_normalized,
                    );
                }
            }
            if !modulation_gesture
                && (response.drag_started() || response.dragged())
                && let Some(position) = response.interact_pointer_pos()
            {
                let x = ((position.x - unison_plot.left()) / unison_plot.width()).clamp(0.0, 1.0);
                let y =
                    ((unison_plot.bottom() - position.y) / unison_plot.height()).clamp(0.0, 1.0);
                traced_set(
                    state,
                    "compact-unison",
                    "set-x-enter",
                    params.detune_amount,
                    x,
                    x,
                    y,
                );
                traced_set(
                    state,
                    "compact-unison",
                    "set-y-enter",
                    params.curve,
                    y,
                    x,
                    y,
                );
            }
            if !modulation_gesture && response.drag_stopped() {
                for id in [params.detune_amount, params.curve] {
                    traced_end(
                        state,
                        "compact-unison",
                        id,
                        state.get_param(params.detune_amount),
                        state.get_param(params.curve),
                    );
                }
            }

            let alignment_response = ui
                .interact(
                    alignment_rail.expand2(egui::vec2(2.0, 0.0)),
                    outer.id.with("alignment-amount"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeVertical)
                .on_hover_text("Unison alignment amount");
            let alignment_modulation = editor_modulation::owns_gesture(
                ui,
                state,
                params.harmonic_align,
                &alignment_response,
            );
            if !alignment_modulation && alignment_response.drag_started() {
                state.begin_edit(params.harmonic_align);
            }
            if !alignment_modulation
                && (alignment_response.drag_started() || alignment_response.dragged())
                && let Some(pointer) = alignment_response.interact_pointer_pos()
            {
                let value = ((alignment_rail.bottom() - pointer.y) / alignment_rail.height())
                    .clamp(0.0, 1.0);
                if (state.get_param(params.harmonic_align) - value).abs() > f32::EPSILON {
                    state.set_param(params.harmonic_align, f64::from(value));
                }
            }
            if !alignment_modulation && alignment_response.drag_stopped() {
                state.end_edit(params.harmonic_align);
            }
            if !alignment_modulation
                && alignment_response.clicked()
                && let Some(pointer) = alignment_response.interact_pointer_pos()
            {
                let value = ((alignment_rail.bottom() - pointer.y) / alignment_rail.height())
                    .clamp(0.0, 1.0);
                state.begin_edit(params.harmonic_align);
                state.set_param(params.harmonic_align, f64::from(value));
                state.end_edit(params.harmonic_align);
            }
            paint_compact_alignment_rail(
                &painter,
                alignment_rail,
                harmonic_align,
                alignment_response.hovered(),
            );
            paint_compact_distribution(
                &painter,
                &points[..usize::from(voices)],
                &weights[..usize::from(voices)],
                maximum_weight,
                egui::pos2(
                    egui::lerp(
                        unison_plot.left()..=unison_plot.right(),
                        detune_amount_normalized,
                    ),
                    egui::lerp(unison_plot.bottom()..=unison_plot.top(), curve_normalized),
                ),
                1.0,
            );
            editor_modulation::destination_xy(
                ui,
                state,
                params.detune_amount,
                params.curve,
                &response,
                unison_plot,
            );
            editor_modulation::destination(
                ui,
                state,
                params.harmonic_align,
                &alignment_response,
                state.get_param(params.harmonic_align),
                alignment_rail,
                editor_modulation::TrackAxis::Vertical,
            );
        }
        CompactUnisonView::PanShape => {
            let (pan_shape_rect, stereo_rect) = compact_pan_shape_panes(view_rect);
            paint_compact_distribution(
                &painter,
                &points[..usize::from(voices)],
                &weights[..usize::from(voices)],
                maximum_weight,
                egui::pos2(
                    egui::lerp(
                        unison_plot.left()..=unison_plot.right(),
                        detune_amount_normalized,
                    ),
                    egui::lerp(unison_plot.bottom()..=unison_plot.top(), curve_normalized),
                ),
                0.13,
            );
            paint_compact_pan_shape_divider(&painter, view_rect);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(outer.id.with("pan-shape-editor"))
                    .max_rect(pan_shape_rect),
            );
            child.set_clip_rect(ui.clip_rect().intersect(pan_shape_rect));
            pan_shape_view(
                &mut child,
                state,
                pan_shape_rect.width(),
                pan_shape_rect.height(),
                params,
                false,
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(outer.id.with("stereo-square-editor"))
                    .max_rect(stereo_rect),
            );
            child.set_clip_rect(ui.clip_rect().intersect(stereo_rect));
            let _ = stereo_square_view(
                &mut child,
                state,
                stereo_rect.width(),
                stereo_rect.height(),
                params,
            );
        }
    }
}

pub(crate) fn custom_unison_view(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    config: &mut crate::generators::OscillatorConfig,
) -> bool {
    let (outer, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::hover(),
    );
    let (header, view_rect, unison_plot, alignment_rail) = compact_unison_layout(outer.rect);
    let before = (
        config.unison_amount.to_bits(),
        config.unison_curve.to_bits(),
        config.unison_alignment.to_bits(),
        config.unison_alignment_mode,
        config.unison_pan_curve.to_bits(),
        config.unison_stereo_x.to_bits(),
        config.unison_stereo_alternate.to_bits(),
    );
    painter.rect_filled(outer.rect, 0.0, editor_theme::semantic().well);
    let view_id = outer.id.with("view");
    let current_view = ui
        .data(|data| data.get_temp::<CompactUnisonView>(view_id))
        .unwrap_or_default();
    let selected_view = compact_unison_view_tabs(ui, &painter, header, view_id, current_view);
    match selected_view {
        CompactUnisonView::Unison => {
            let alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
            let mode_width = (header.width() * 0.30).clamp(56.0, 70.0);
            let mode_rect = egui::Rect::from_min_max(
                egui::pos2(
                    (header.right() - mode_width).max(header.left()),
                    header.top(),
                ),
                header.max,
            );
            painter.text(
                egui::pos2(
                    (header.left() + 118.0).min(mode_rect.left()),
                    header.center().y,
                ),
                egui::Align2::LEFT_CENTER,
                format!(
                    "ALIGN {:.0}%",
                    config.unison_alignment.clamp(0.0, 1.0) * 100.0
                ),
                editor_theme::font::caption(),
                editor_theme::semantic().unison,
            );
            if let Some(mode) = compact_alignment_mode_combo(
                ui,
                mode_rect,
                outer.id.with("alignment-mode"),
                alignment_mode,
            ) {
                config.unison_alignment_mode = mode.index();
            }

            let response = ui
                .interact(
                    unison_plot,
                    outer.id.with("distribution"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Crosshair)
                .on_hover_text("Drag X for detune amount; drag Y for distribution curve");
            if (response.drag_started() || response.dragged())
                && let Some(position) = response.interact_pointer_pos()
            {
                config.unison_amount =
                    ((position.x - unison_plot.left()) / unison_plot.width()).clamp(0.0, 1.0);
                config.unison_curve = ((unison_plot.bottom() - position.y) / unison_plot.height())
                    .clamp(0.0, 1.0)
                    .mul_add(2.0, -1.0);
            } else if response.double_clicked() {
                config.unison_amount = 1.0;
                config.unison_curve = 0.432_959_4;
            }

            let response = ui
                .interact(
                    alignment_rail.expand2(egui::vec2(2.0, 0.0)),
                    outer.id.with("alignment-amount"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeVertical)
                .on_hover_text("Unison alignment amount");
            if (response.drag_started() || response.dragged() || response.clicked())
                && let Some(pointer) = response.interact_pointer_pos()
            {
                config.unison_alignment = ((alignment_rail.bottom() - pointer.y)
                    / alignment_rail.height())
                .clamp(0.0, 1.0);
            }
            paint_compact_alignment_rail(
                &painter,
                alignment_rail,
                config.unison_alignment,
                response.hovered(),
            );
        }
        CompactUnisonView::PanShape => {
            let (pan_shape_rect, stereo_rect) = compact_pan_shape_panes(view_rect);
            let pan_plot = pan_shape_rect.shrink2(egui::vec2(5.0, 3.0));
            let response = ui
                .interact(
                    pan_shape_rect,
                    outer.id.with("pan-shape"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text("Pan distribution curve");
            if response.double_clicked() {
                config.unison_pan_curve = 0.0;
            } else if (response.drag_started() || response.dragged() || response.clicked())
                && let Some(pointer) = response.interact_pointer_pos()
            {
                config.unison_pan_curve = ((pointer.x - pan_plot.left()) / pan_plot.width())
                    .clamp(0.0, 1.0)
                    .mul_add(2.0, -1.0);
            }
            custom_stereo_square_view(
                ui,
                &painter,
                stereo_rect,
                outer.id.with("stereo-square"),
                &mut config.unison_stereo_x,
                &mut config.unison_stereo_alternate,
            );
        }
    }

    let alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
    let voices_u8 = config.unison_voices.clamp(1, MAX_UNISON as u8);
    let voices = usize::from(voices_u8);
    let rate = normalized_unison_rate(config.unison_rate);
    let time = ui.input(|input| input.time) as f32 * rate;
    if config.unison_jitter > f32::EPSILON {
        editor_theme::request_display_repaint(ui);
    }
    let full_scale =
        (config.unison_range * 100.0 + JITTER_EXCURSION_CENTS * config.unison_jitter).max(1.0);
    let mut points = [egui::Pos2::ZERO; MAX_UNISON];
    let weights = [1.0_f32; MAX_UNISON];
    let mut jitter_offsets = [0.0_f32; MAX_UNISON];
    fill_extended_unison_jitter_offsets(
        &mut jitter_offsets[..voices],
        0.618_034,
        config.unison_jitter,
        time,
    );
    let pan_shape = PanShapeSettings::symmetric_curve(config.unison_pan_curve);
    let spatial_settings = UnisonSettings::new(
        voices_u8,
        config.unison_range * 100.0,
        config.unison_width,
        config.phase_random,
        config.unison_curve,
    )
    .with_stereo_square(config.unison_stereo_alternate, config.unison_stereo_x)
    .with_pan_shape(pan_shape);
    let mut detune_positions = [0.0_f32; MAX_UNISON];
    let mut lane_left = [0.0_f32; MAX_UNISON];
    let mut lane_right = [0.0_f32; MAX_UNISON];
    fill_extended_unison_layout(
        spatial_settings,
        &mut detune_positions,
        &mut lane_left,
        &mut lane_right,
    );
    for (index, point) in points[..voices].iter_mut().enumerate() {
        let detune = unison_static_pitch_cents(
            detune_positions[index],
            config.unison_range * 100.0,
            config.unison_amount,
            config.unison_alignment,
            alignment_mode,
        );
        let jitter = jitter_offsets[index] * JITTER_EXCURSION_CENTS;
        let left_energy = lane_left[index] * lane_left[index];
        let right_energy = lane_right[index] * lane_right[index];
        let pan = (right_energy - left_energy) / (right_energy + left_energy).max(f32::EPSILON);
        *point = egui::pos2(
            ((detune + jitter) / full_scale)
                .mul_add(unison_plot.width() * 0.46, unison_plot.center().x),
            (-pan).mul_add(unison_plot.height() * 0.38, unison_plot.center().y),
        );
    }
    match selected_view {
        CompactUnisonView::Unison => {
            paint_compact_distribution(
                &painter,
                &points[..voices],
                &weights[..voices],
                1.0,
                egui::pos2(
                    egui::lerp(
                        unison_plot.left()..=unison_plot.right(),
                        config.unison_amount,
                    ),
                    egui::lerp(
                        unison_plot.bottom()..=unison_plot.top(),
                        config.unison_curve.mul_add(0.5, 0.5),
                    ),
                ),
                1.0,
            );
        }
        CompactUnisonView::PanShape => {
            let (pan_shape_rect, _) = compact_pan_shape_panes(view_rect);
            paint_compact_distribution(
                &painter,
                &points[..voices],
                &weights[..voices],
                1.0,
                egui::pos2(
                    egui::lerp(
                        unison_plot.left()..=unison_plot.right(),
                        config.unison_amount,
                    ),
                    egui::lerp(
                        unison_plot.bottom()..=unison_plot.top(),
                        config.unison_curve.mul_add(0.5, 0.5),
                    ),
                ),
                0.13,
            );
            paint_compact_pan_shape_divider(&painter, view_rect);
            paint_compact_pan_shape(
                &painter,
                pan_shape_rect,
                0.5,
                0.0,
                &format!("PAN {:+.2}", config.unison_pan_curve),
                |left, input| {
                    if left {
                        pan_shape.left_segments.eval(input)
                    } else {
                        pan_shape.right_segments.eval(input)
                    }
                },
            );
        }
    }
    before
        != (
            config.unison_amount.to_bits(),
            config.unison_curve.to_bits(),
            config.unison_alignment.to_bits(),
            config.unison_alignment_mode,
            config.unison_pan_curve.to_bits(),
            config.unison_stereo_x.to_bits(),
            config.unison_stereo_alternate.to_bits(),
        )
}

pub(crate) fn normalized_unison_rate(normalized: f32) -> f32 {
    0.02 * 5_000.0_f32.powf(normalized.clamp(0.0, 1.0))
}

fn paint_compact_distribution(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    weights: &[f32],
    maximum_weight: f32,
    control_point: egui::Pos2,
    opacity: f32,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    for (point, weight) in points.iter().zip(weights) {
        let relative = (weight / maximum_weight.max(f32::EPSILON)).sqrt();
        let half_height = relative.mul_add(10.0, 4.0);
        let color = editor_theme::semantic()
            .unison
            .linear_multiply(relative.mul_add(0.72, 0.28) * opacity);
        painter.line_segment(
            [
                *point - egui::vec2(0.0, half_height),
                *point + egui::vec2(0.0, half_height),
            ],
            egui::Stroke::new(1.8_f32, color),
        );
    }
    painter.circle_filled(
        control_point,
        3.5,
        editor_theme::semantic().unison.linear_multiply(opacity),
    );
}

#[derive(Clone, Copy)]
struct StereoSquare {
    rect: egui::Rect,
}

impl StereoSquare {
    fn new(rect: egui::Rect) -> Self {
        Self { rect }
    }

    fn point(self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(
            egui::lerp(self.rect.left()..=self.rect.right(), x.clamp(0.0, 1.0)),
            egui::lerp(self.rect.bottom()..=self.rect.top(), y.clamp(0.0, 1.0)),
        )
    }

    fn axes_at(self, point: egui::Pos2) -> (f32, f32) {
        let x = ((point.x - self.rect.left()) / self.rect.width()).clamp(0.0, 1.0);
        let y = ((self.rect.bottom() - point.y) / self.rect.height()).clamp(0.0, 1.0);
        (x, y)
    }

    fn snap(self, axes: (f32, f32), enabled: bool) -> (f32, f32) {
        if !enabled {
            return axes;
        }
        let candidates = [
            (0.0, 1.0),
            (1.0, 1.0),
            (0.0, 0.0),
            (1.0, 0.0),
            (0.5, 1.0),
            (0.5, 0.0),
            (0.0, 0.5),
            (1.0, 0.5),
            (0.5, 0.5),
        ];
        let point = self.point(axes.0, axes.1);
        candidates
            .into_iter()
            .filter(|candidate| self.point(candidate.0, candidate.1).distance(point) <= 9.0)
            .min_by(|left, right| {
                self.point(left.0, left.1)
                    .distance_sq(point)
                    .total_cmp(&self.point(right.0, right.1).distance_sq(point))
            })
            .unwrap_or(axes)
    }
}

fn stereo_square_plot(rect: egui::Rect) -> egui::Rect {
    rect.shrink((rect.width() * 0.055).clamp(3.0, 6.0))
}

fn paint_stereo_square(painter: &egui::Painter, plot: egui::Rect, x: f32, y: f32) {
    let palette = editor_theme::semantic();
    let accent = palette.pan_shape;
    painter.rect(
        plot,
        2.0,
        plugcat::theme::mix(palette.well, accent, 0.08),
        egui::Stroke::new(1.0_f32, accent),
        egui::StrokeKind::Inside,
    );
    let guide = egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.32));
    painter.line_segment(
        [
            egui::pos2(plot.center().x, plot.top()),
            egui::pos2(plot.center().x, plot.bottom()),
        ],
        guide,
    );
    painter.line_segment(
        [
            egui::pos2(plot.left(), plot.center().y),
            egui::pos2(plot.right(), plot.center().y),
        ],
        guide,
    );
    let point = StereoSquare::new(plot).point(x, y);
    let point_radius = (plot.width() * 0.055).clamp(3.5, 5.5);
    painter.circle_filled(point, point_radius, accent);
    painter.circle_stroke(point, point_radius, egui::Stroke::new(1.0_f32, accent));
    let compact = plot.width() < 80.0;
    for (position, align, compact_label, label) in [
        (
            plot.left_top() + egui::vec2(6.0, 5.0),
            egui::Align2::LEFT_TOP,
            "A",
            "ALTR",
        ),
        (
            plot.right_top() + egui::vec2(-6.0, 5.0),
            egui::Align2::RIGHT_TOP,
            "P",
            "PAIR",
        ),
        (
            plot.left_bottom() + egui::vec2(6.0, -5.0),
            egui::Align2::LEFT_BOTTOM,
            "R",
            "RAND",
        ),
        (
            plot.right_bottom() + egui::vec2(-6.0, -5.0),
            egui::Align2::RIGHT_BOTTOM,
            "S",
            "SHAP",
        ),
    ] {
        painter.text(
            position,
            align,
            if compact { compact_label } else { label },
            editor_theme::font::caption(),
            accent,
        );
    }
}

fn custom_stereo_square_view(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    id: egui::Id,
    x: &mut f32,
    y: &mut f32,
) {
    let response = ui
        .interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Crosshair)
        .on_hover_text("X selects stereo pattern; Y blends alternate/pair with random/shape");
    let plot = stereo_square_plot(rect);
    if (response.drag_started_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary))
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let anchor = pointer - response.drag_delta();
        let (constrained, snapping) = ui.input(|input| {
            (
                constrain_drag(anchor, pointer, input.modifiers.alt),
                !input.modifiers.shift,
            )
        });
        (*x, *y) =
            StereoSquare::new(plot).snap(StereoSquare::new(plot).axes_at(constrained), snapping);
    }
    *x = (*x).clamp(0.0, 1.0);
    *y = (*y).clamp(0.0, 1.0);
    paint_stereo_square(painter, plot, *x, *y);
}

pub(crate) fn stereo_square_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    params: UnisonUiParams,
) -> Option<bool> {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
    let rect = response.rect;
    let plot = stereo_square_plot(rect);
    let square = StereoSquare::new(plot);
    let drag_active = response.drag_started_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary)
        || response.drag_stopped_by(egui::PointerButton::Primary);
    if drag_active {
        crate::diagnostics::trace(
            "stereo-square",
            "frame-enter",
            plain_param_value(state, params.stereo_x),
            plain_param_value(state, params.stereo_alternate),
        );
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        let x = plain_param_value(state, params.stereo_x).clamp(0.0, 1.0);
        let y = plain_param_value(state, params.stereo_alternate).clamp(0.0, 1.0);
        traced_begin(state, "stereo-square", params.stereo_x, x, y);
    }
    let mut requested_shaper = None;
    if (response.drag_started_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary))
        && let Some(pointer) = response.interact_pointer_pos()
    {
        crate::diagnostics::trace("stereo-square", "input-enter", pointer.x, pointer.y);
        let anchor = pointer - response.drag_delta();
        let (constrained, snapping) = ui.input(|input| {
            (
                constrain_drag(anchor, pointer, input.modifiers.alt),
                !input.modifiers.shift,
            )
        });
        let (x, y) = square.snap(square.axes_at(constrained), snapping);
        let shape_weight = x * (1.0 - y);
        requested_shaper = if shape_weight >= 0.90 {
            Some(true)
        } else if shape_weight <= 0.72 {
            Some(false)
        } else {
            None
        };
        crate::diagnostics::trace("stereo-square", "input-return", x, y);
        traced_set(
            state,
            "stereo-square",
            "set-x-enter",
            params.stereo_x,
            x,
            x,
            y,
        );
        traced_set(
            state,
            "stereo-square",
            "set-y-enter",
            params.stereo_alternate,
            y,
            x,
            y,
        );
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        traced_end(
            state,
            "stereo-square",
            params.stereo_x,
            plain_param_value(state, params.stereo_x),
            plain_param_value(state, params.stereo_alternate),
        );
    }

    if drag_active {
        crate::diagnostics::trace(
            "stereo-square",
            "input-phase-return",
            plain_param_value(state, params.stereo_x),
            plain_param_value(state, params.stereo_alternate),
        );
    }

    let x = editor_modulation::effective_plain_value(state, params.stereo_x).clamp(0.0, 1.0);
    let y =
        editor_modulation::effective_plain_value(state, params.stereo_alternate).clamp(0.0, 1.0);
    paint_stereo_square(&painter, plot, x, y);
    if drag_active {
        crate::diagnostics::trace("stereo-square", "paint-return", x, y);
    }
    editor_modulation::destination_xy(
        ui,
        state,
        params.stereo_x,
        params.stereo_alternate,
        &response,
        plot,
    );
    requested_shaper
}

/// Shape uses a direct bounded point-curve contract: center and edges remain
/// anchors while interior knots and per-segment bend handles stay editable.
pub(crate) fn pan_shape_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
    binding: UnisonUiParams,
    clear_background: bool,
) {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
    let rect = response.rect;
    let plot = if clear_background {
        editor_widgets::graph_plot(
            rect,
            ui,
            editor_theme::title_height(ui),
            editor_theme::space::SM,
        )
    } else {
        rect.shrink2(egui::vec2(5.0, 3.0))
    };
    let params = state.params();
    let curve_state = pan_shape_curve_state(params, binding);
    let mut center_x =
        editor_modulation::effective_plain_value(state, binding.pan_center_x).clamp(0.05, 0.95);
    let mut data = if curve_state.is_initialized() {
        curve_state.snapshot()
    } else {
        PanShapeCurveData::from_legacy(
            editor_modulation::effective_plain_value(state, binding.pan_center),
            editor_modulation::effective_plain_value(state, binding.pan_left),
            editor_modulation::effective_plain_value(state, binding.pan_right),
            plain_param_value(state, binding.pan_left_curve),
            plain_param_value(state, binding.pan_right_curve),
            plain_param_value(state, binding.pan_left_curve_time),
            plain_param_value(state, binding.pan_right_curve_time),
        )
    };
    if curve_state.is_initialized() {
        let center = editor_modulation::effective_plain_value(state, binding.pan_center);
        let left = editor_modulation::effective_plain_value(state, binding.pan_left);
        let right = editor_modulation::effective_plain_value(state, binding.pan_right);
        if let Some(knot) = data.left.knots.first_mut() {
            knot.out_lin = center;
        }
        if let Some(knot) = data.left.knots.last_mut() {
            knot.out_lin = left;
        }
        if let Some(knot) = data.right.knots.last_mut() {
            knot.out_lin = right;
        }
    }
    if !curve_state.is_initialized() {
        curve_state.replace(data.clone());
    }
    let drag_id = response.id.with("pan_shape_point_drag");
    let pointer = response.interact_pointer_pos();
    let mut active = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id));
    if active.is_none()
        && response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer.filter(|pointer| plot.contains(*pointer))
        && !pan_shape_hit_any(&data, plot, center_x, pointer)
    {
        let (left, input, output) = pan_shape_values_from_pos(plot, center_x, pointer);
        let mirror = ui.input(|input| input.modifiers.shift);
        let inserted = curve_state.edit(|curve| {
            let mut candidate = curve.clone();
            if !insert_knot(candidate.half_mut(left), input, output)
                || (mirror && !insert_knot(candidate.half_mut(!left), input, output))
            {
                return false;
            }
            *curve = candidate;
            true
        });
        if inserted {
            data = curve_state.snapshot();
            editor_theme::request_display_repaint(ui);
        }
    }
    if active.is_none()
        && response.clicked_by(egui::PointerButton::Secondary)
        && let Some(pointer) = pointer
        && let Some((left, index)) = pan_shape_hit_knot(&data, plot, center_x, pointer)
    {
        let mirror = ui.input(|input| input.modifiers.shift);
        let mirror_index = mirror
            .then(|| matching_knot_index(data.half(!left), data.half(left).knots[index].in_lin));
        let removed = curve_state.edit(|curve| {
            let mut candidate = curve.clone();
            if !remove_knot(candidate.half_mut(left), index)
                || (mirror
                    && !mirror_index
                        .flatten()
                        .is_some_and(|index| remove_knot(candidate.half_mut(!left), index)))
            {
                return false;
            }
            *curve = candidate;
            true
        });
        if removed {
            data = curve_state.snapshot();
            editor_theme::request_display_repaint(ui);
        }
    }
    if active.is_none()
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer
    {
        let target = if pan_shape_hit_center(&data, plot, center_x, pointer) {
            Some(PanShapePointDragTarget::Center)
        } else if pointer.distance(pan_shape_endpoint(&data, plot, center_x, true)) <= 12.0 {
            Some(PanShapePointDragTarget::Endpoint { left: true })
        } else if pointer.distance(pan_shape_endpoint(&data, plot, center_x, false)) <= 12.0 {
            Some(PanShapePointDragTarget::Endpoint { left: false })
        } else if let Some((left, index)) = pan_shape_hit_knot(&data, plot, center_x, pointer) {
            Some(PanShapePointDragTarget::Knot { left, index })
        } else if let Some((left, index)) = pan_shape_hit_curve(&data, plot, center_x, pointer) {
            Some(PanShapePointDragTarget::Curve { left, index })
        } else {
            None
        };
        if let Some(target) = target {
            match target {
                PanShapePointDragTarget::Center => {
                    for id in [binding.pan_center_x, binding.pan_center] {
                        traced_begin(
                            state,
                            "pan-shape-center",
                            id,
                            center_x,
                            plain_param_value(state, binding.pan_center),
                        );
                    }
                }
                PanShapePointDragTarget::Endpoint { .. } => {
                    for id in [binding.pan_left, binding.pan_right] {
                        traced_begin(state, "pan-shape-edge", id, center_x, pointer.y);
                    }
                }
                PanShapePointDragTarget::Knot { .. } | PanShapePointDragTarget::Curve { .. } => {}
            }
            ui.data_mut(|store| {
                store.insert_temp(
                    drag_id,
                    PanShapePointDrag {
                        target,
                        anchor: pan_shape_target_pos(&data, plot, center_x, target),
                    },
                );
            });
        }
    }

    if let Some(active) = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id))
        && response.dragged_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer
    {
        let (pointer, mirror) = ui.input(|input| {
            (
                constrain_drag(
                    active.anchor,
                    pointer,
                    input.modifiers.alt
                        && !matches!(active.target, PanShapePointDragTarget::Endpoint { .. }),
                ),
                input.modifiers.shift,
            )
        });
        let mut center_update = None;
        let mut endpoint_update = None;
        curve_state.edit(|curve| match active.target {
            PanShapePointDragTarget::Center => {
                let (_, _, output) = pan_shape_values_from_pos(plot, center_x, pointer);
                let normalized_x =
                    ((pointer.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0);
                move_center(curve, output);
                center_update = Some((output, normalized_x));
                center_x = normalized_x.mul_add(0.9, 0.05);
            }
            PanShapePointDragTarget::Endpoint { left } => {
                let (_, output) = pan_shape_values_from_side(plot, center_x, left, pointer);
                move_endpoint(curve.half_mut(left), output);
                endpoint_update = Some((left, output, mirror));
                if mirror {
                    move_endpoint(curve.half_mut(!left), output);
                }
            }
            PanShapePointDragTarget::Knot { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, center_x, left, pointer);
                let mirror_index = mirror.then(|| {
                    matching_knot_index(curve.half(!left), curve.half(left).knots[index].in_lin)
                });
                move_knot(curve.half_mut(left), index, input, output);
                if let Some(Some(index)) = mirror_index {
                    move_knot(curve.half_mut(!left), index, input, output);
                }
            }
            PanShapePointDragTarget::Curve { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, center_x, left, pointer);
                let mirror_index = mirror.then(|| {
                    let half = curve.half(left);
                    let center = (half.knots[index].in_lin + half.knots[index + 1].in_lin) * 0.5;
                    matching_segment_index(curve.half(!left), center)
                });
                let half = curve.half_mut(left);
                let start = half.knots[index].out_lin;
                let end = half.knots[index + 1].out_lin;
                let segment_start = half.knots[index].in_lin;
                let segment_end = half.knots[index + 1].in_lin;
                let level = if (end - start).abs() > f32::EPSILON {
                    ((output - start) / (end - start)).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                let vertical = level.mul_add(2.0, -1.0);
                let segment_input = ((input - segment_start)
                    / (segment_end - segment_start).max(f32::EPSILON))
                .clamp(0.0, 1.0);
                let horizontal = ((segment_input - 0.5) / 0.44).clamp(-1.0, 1.0);
                set_segment_curve(half, index, vertical, horizontal);
                if let Some(Some(index)) = mirror_index {
                    set_segment_curve(curve.half_mut(!left), index, vertical, horizontal);
                }
            }
        });
        if let Some((output, normalized_x)) = center_update {
            if (plain_param_value(state, binding.pan_center) - output).abs() > f32::EPSILON {
                traced_set(
                    state,
                    "pan-shape-center",
                    "set-y-enter",
                    binding.pan_center,
                    output,
                    normalized_x,
                    output,
                );
            }
            if (plain_param_value(state, binding.pan_center_x) - normalized_x).abs() > f32::EPSILON
            {
                traced_set(
                    state,
                    "pan-shape-center",
                    "set-x-enter",
                    binding.pan_center_x,
                    normalized_x,
                    normalized_x,
                    output,
                );
            }
        }
        if let Some((left, output, mirror)) = endpoint_update {
            for side in [left, !left].into_iter().take(if mirror { 2 } else { 1 }) {
                let id = if side {
                    binding.pan_left
                } else {
                    binding.pan_right
                };
                if (plain_param_value(state, id) - output).abs() > f32::EPSILON {
                    traced_set(
                        state,
                        "pan-shape-edge",
                        "set-y-enter",
                        id,
                        output,
                        pointer.x,
                        output,
                    );
                }
            }
        }
        ui.data_mut(|store| store.insert_temp(drag_id, active));
        data = curve_state.snapshot();
        editor_theme::request_display_repaint(ui);
    }
    active = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id));
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        if let Some(drag) = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id)) {
            match drag.target {
                PanShapePointDragTarget::Center => {
                    for id in [binding.pan_center_x, binding.pan_center] {
                        traced_end(
                            state,
                            "pan-shape-center",
                            id,
                            plain_param_value(state, binding.pan_center_x),
                            plain_param_value(state, binding.pan_center),
                        );
                    }
                }
                PanShapePointDragTarget::Endpoint { .. } => {
                    for id in [binding.pan_left, binding.pan_right] {
                        traced_end(
                            state,
                            "pan-shape-edge",
                            id,
                            plain_param_value(state, binding.pan_center_x),
                            plain_param_value(state, id),
                        );
                    }
                }
                PanShapePointDragTarget::Knot { .. } | PanShapePointDragTarget::Curve { .. } => {}
            }
        }
        ui.data_mut(|store| store.remove::<PanShapePointDrag>(drag_id));
        active = None;
    }
    draw_pan_shape_curve(
        &painter,
        rect,
        plot,
        center_x,
        &data,
        pointer,
        active,
        clear_background,
    );
    editor_modulation::destination_xy(
        ui,
        state,
        binding.pan_center_x,
        binding.pan_center,
        &response,
        plot,
    );
}

fn matching_knot_index(half: &crate::pan_curve::PanShapeHalf, input: f32) -> Option<usize> {
    half.knots
        .iter()
        .enumerate()
        .skip(1)
        .take(half.knots.len().saturating_sub(2))
        .min_by(|(_, left), (_, right)| {
            (left.in_lin - input)
                .abs()
                .total_cmp(&(right.in_lin - input).abs())
        })
        .map(|(index, _)| index)
}

fn matching_segment_index(half: &crate::pan_curve::PanShapeHalf, input: f32) -> Option<usize> {
    half.knots
        .windows(2)
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            let left = ((left[0].in_lin + left[1].in_lin) * 0.5 - input).abs();
            let right = ((right[0].in_lin + right[1].in_lin) * 0.5 - input).abs();
            left.total_cmp(&right)
        })
        .map(|(index, _)| index)
}

fn draw_pan_shape_curve(
    painter: &egui::Painter,
    rect: egui::Rect,
    plot: egui::Rect,
    center_x: f32,
    data: &PanShapeCurveData,
    pointer: Option<egui::Pos2>,
    drag: Option<PanShapePointDrag>,
    clear_background: bool,
) {
    let color = editor_theme::semantic().pan_shape;
    if clear_background {
        editor_widgets::graph_frame(painter, rect);
        editor_widgets::graph_title(painter, rect, "PAN SHAPE");
        let grid = egui::Stroke::new(1.0_f32, editor_theme::semantic().grid);
        painter.line_segment([plot.left_bottom(), plot.right_bottom()], grid);
        let center_line_x = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
        painter.line_segment(
            [
                egui::pos2(center_line_x, plot.top()),
                egui::pos2(center_line_x, plot.bottom()),
            ],
            grid,
        );
        painter.text(
            plot.left_top() + egui::vec2(0.0, 4.0),
            egui::Align2::LEFT_TOP,
            "L",
            editor_theme::font::label(),
            editor_theme::semantic().text_muted,
        );
        painter.text(
            plot.right_top() + egui::vec2(0.0, 4.0),
            egui::Align2::RIGHT_TOP,
            "R",
            editor_theme::font::label(),
            editor_theme::semantic().text_muted,
        );
    }
    let center_line_x = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let draw_half = |left: bool| -> Vec<egui::Pos2> {
        let segments = data.half(left).compile_rt();
        (0..=CURVE_POINTS)
            .map(|index| {
                let input = f32::from(index) / f32::from(CURVE_POINTS);
                let x = if left {
                    egui::lerp(center_line_x..=plot.left(), input)
                } else {
                    egui::lerp(center_line_x..=plot.right(), input)
                };
                egui::pos2(
                    x,
                    egui::lerp(plot.bottom()..=plot.top(), segments.eval(input)),
                )
            })
            .collect()
    };
    let left_points = draw_half(true);
    let right_points = draw_half(false);
    editor_widgets::gradient_area_to_bottom(painter, &left_points, plot.bottom(), color, 110);
    editor_widgets::gradient_area_to_bottom(painter, &right_points, plot.bottom(), color, 110);
    painter.add(egui::Shape::line(
        left_points,
        egui::Stroke::new(2.0_f32, color),
    ));
    painter.add(egui::Shape::line(
        right_points,
        egui::Stroke::new(2.0_f32, color),
    ));
    for (left, half) in [(true, &data.left), (false, &data.right)] {
        let Some(first) = half.knots.first().copied() else {
            continue;
        };
        let Some(last) = half.knots.last().copied() else {
            continue;
        };
        let center_active = drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.target, PanShapePointDragTarget::Center));
        let endpoint_active = drag.as_ref().is_some_and(|drag| {
            matches!(drag.target, PanShapePointDragTarget::Endpoint { left: side } if side == left)
        });
        let center = pan_shape_knot_pos(plot, center_x, left, first);
        let endpoint = pan_shape_knot_pos(plot, center_x, left, last);
        let center_hover = pointer.is_some_and(|pointer| pointer.distance(center) <= 12.0);
        let endpoint_hover = pointer.is_some_and(|pointer| pointer.distance(endpoint) <= 12.0);
        if left {
            draw_shape_handle(painter, center, color, center_active || center_hover, false);
        }
        draw_shape_handle(
            painter,
            endpoint,
            color,
            endpoint_active || endpoint_hover,
            false,
        );

        for (index, knot) in half
            .knots
            .iter()
            .copied()
            .enumerate()
            .skip(1)
            .take(half.knots.len().saturating_sub(2))
        {
            let position = pan_shape_knot_pos(plot, center_x, left, knot);
            let knot_active = drag.as_ref().is_some_and(|drag| {
                matches!(drag.target, PanShapePointDragTarget::Knot { left: side, index: target } if side == left && target == index)
            });
            let knot_hover = pointer.is_some_and(|pointer| pointer.distance(position) <= 12.0);
            draw_shape_handle(painter, position, color, knot_active || knot_hover, false);
        }

        for index in 0..half.knots.len().saturating_sub(1) {
            let curve = pan_shape_curve_handle_pos(plot, center_x, left, half, index);
            let curve_active = drag.as_ref().is_some_and(|drag| {
                matches!(drag.target, PanShapePointDragTarget::Curve { left: side, index: target } if side == left && target == index)
            });
            let curve_hover = pointer.is_some_and(|pointer| pointer.distance(curve) <= 12.0);
            draw_shape_handle(painter, curve, color, curve_active || curve_hover, true);
        }
    }
}

fn draw_shape_handle(
    painter: &egui::Painter,
    position: egui::Pos2,
    color: egui::Color32,
    highlighted: bool,
    curve: bool,
) {
    let radius = if curve { 3.5 } else { 4.0 } + if highlighted { 1.0 } else { 0.0 };
    painter.circle_filled(
        position,
        radius,
        if curve {
            editor_theme::semantic().surface
        } else {
            color
        },
    );
    painter.circle_stroke(position, radius, egui::Stroke::new(1.25_f32, color));
}

fn pan_shape_curve_handle_pos(
    plot: egui::Rect,
    center_x: f32,
    left: bool,
    half: &crate::pan_curve::PanShapeHalf,
    index: usize,
) -> egui::Pos2 {
    let Some(start) = half.knots.get(index).copied() else {
        return plot.center();
    };
    let Some(end) = half.knots.get(index + 1).copied() else {
        return pan_shape_knot_pos(plot, center_x, left, start);
    };
    let segments = half.compile_rt();
    let y = segments.seg_p1[index].clamp(0.0, 1.0);
    editor_envelope::curve_handle_position(
        pan_shape_knot_pos(plot, center_x, left, start),
        pan_shape_knot_pos(plot, center_x, left, end),
        segments.seg_cx1[index],
        y,
    )
}

fn pan_shape_knot_pos(
    plot: egui::Rect,
    center_x: f32,
    left: bool,
    knot: PanShapeKnot,
) -> egui::Pos2 {
    let center = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let x = if left {
        egui::lerp(center..=plot.left(), knot.in_lin)
    } else {
        egui::lerp(center..=plot.right(), knot.in_lin)
    };
    egui::pos2(x, egui::lerp(plot.bottom()..=plot.top(), knot.out_lin))
}

fn pan_shape_endpoint(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    left: bool,
) -> egui::Pos2 {
    let knot = data.half(left).knots.last().copied().unwrap_or_default();
    pan_shape_knot_pos(plot, center_x, left, knot)
}

fn pan_shape_target_pos(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    target: PanShapePointDragTarget,
) -> egui::Pos2 {
    match target {
        PanShapePointDragTarget::Center => data
            .left
            .knots
            .first()
            .copied()
            .map_or(plot.center(), |knot| {
                pan_shape_knot_pos(plot, center_x, true, knot)
            }),
        PanShapePointDragTarget::Endpoint { left } => {
            pan_shape_endpoint(data, plot, center_x, left)
        }
        PanShapePointDragTarget::Knot { left, index } => data
            .half(left)
            .knots
            .get(index)
            .copied()
            .map_or(plot.center(), |knot| {
                pan_shape_knot_pos(plot, center_x, left, knot)
            }),
        PanShapePointDragTarget::Curve { left, index } => {
            pan_shape_curve_handle_pos(plot, center_x, left, data.half(left), index)
        }
    }
}

fn pan_shape_hit_center(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
) -> bool {
    data.left.knots.first().is_some_and(|knot| {
        pointer.distance(pan_shape_knot_pos(plot, center_x, true, *knot)) <= 12.0
    })
}

fn pan_shape_hit_curve(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
) -> Option<(bool, usize)> {
    for (left, half) in [(true, &data.left), (false, &data.right)] {
        for index in 0..half.knots.len().saturating_sub(1) {
            let handle = pan_shape_curve_handle_pos(plot, center_x, left, half, index);
            if pointer.distance(handle) <= 14.0 {
                return Some((left, index));
            }
        }
    }
    None
}

fn pan_shape_hit_knot(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
) -> Option<(bool, usize)> {
    for (left, half) in [(true, &data.left), (false, &data.right)] {
        for (index, knot) in half
            .knots
            .iter()
            .copied()
            .enumerate()
            .skip(1)
            .take(half.knots.len().saturating_sub(2))
        {
            if pointer.distance(pan_shape_knot_pos(plot, center_x, left, knot)) <= 12.0 {
                return Some((left, index));
            }
        }
    }
    None
}

fn pan_shape_hit_any(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
) -> bool {
    pan_shape_hit_center(data, plot, center_x, pointer)
        || pointer.distance(pan_shape_endpoint(data, plot, center_x, true)) <= 12.0
        || pointer.distance(pan_shape_endpoint(data, plot, center_x, false)) <= 12.0
        || pan_shape_hit_knot(data, plot, center_x, pointer).is_some()
        || pan_shape_hit_curve(data, plot, center_x, pointer).is_some()
}

fn pan_shape_values_from_side(
    plot: egui::Rect,
    center_x: f32,
    left: bool,
    pointer: egui::Pos2,
) -> (f32, f32) {
    let center = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let input = if left {
        (center - pointer.x) / (center - plot.left()).max(1.0)
    } else {
        (pointer.x - center) / (plot.right() - center).max(1.0)
    };
    let output = (plot.bottom() - pointer.y) / plot.height().max(1.0);
    (input.clamp(0.0, 1.0), output.clamp(0.0, 1.0))
}

fn pan_shape_values_from_pos(
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
) -> (bool, f32, f32) {
    let center = egui::lerp(plot.left()..=plot.right(), center_x.clamp(0.05, 0.95));
    let left = pointer.x < center;
    let (input, output) = pan_shape_values_from_side(plot, center_x, left, pointer);
    (left, input, output)
}
