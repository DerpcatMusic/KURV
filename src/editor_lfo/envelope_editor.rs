use super::*;

pub(super) fn draw_envelope_curve(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    index: usize,
    width: f32,
    height: f32,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    let plot = rect.shrink(editor_theme::graph_inset(ui));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, editor_theme::semantic().well);
    let editor_id = response.id.with("envelope-editor");
    let mut editor = ui
        .data(|store| store.get_temp::<EnvelopeEditorUi>(editor_id))
        .unwrap_or_default();
    let [attack, decay, sustain, release] = envelope_values(state.params(), index);
    let curves = envelope_curve_values(state.params(), index);
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
    let points = [
        plot.left_bottom(),
        egui::pos2(attack_x, plot.top()),
        egui::pos2(decay_x, sustain_y),
        egui::pos2(sustain_x, sustain_y),
        plot.right_bottom(),
    ];
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
    let handle_radius = (plot.height() * 0.035).clamp(3.5, 6.0);
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let hovered_handle = pointer.and_then(|pointer| {
        handles
            .iter()
            .map(|(stage, position)| (*stage, position.distance_sq(pointer)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .filter(|(_, distance)| *distance <= (handle_radius * 2.6).powi(2))
            .map(|(stage, _)| stage)
    });
    let hovered = hovered_handle.or_else(|| {
        pointer.and_then(|pointer| {
            [
                EnvelopeDrag::Attack,
                EnvelopeDrag::DecaySustain,
                EnvelopeDrag::Sustain,
                EnvelopeDrag::Release,
            ]
            .into_iter()
            .map(|stage| {
                let (start, end) = envelope_segment(&points, stage);
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
            .filter(|(_, distance)| *distance <= (handle_radius * 2.8).powi(2))
            .map(|(stage, _)| stage)
        })
    });

    if response.secondary_clicked() {
        editor.context_target = hovered;
    }
    let context_target = editor.context_target.or(hovered);
    let mut reset_stage = None;
    let mut reset_all = false;
    response.context_menu(|ui| {
        if let Some(stage) = context_target
            && ui
                .button(format!("RESET {}", envelope_stage_label(stage)))
                .clicked()
        {
            reset_stage = Some(stage);
            ui.close();
        }
        if ui.button("RESET ENVELOPE").clicked() {
            reset_all = true;
            ui.close();
        }
    });
    if !response.context_menu_opened() {
        editor.context_target = None;
    }
    if reset_all {
        finish_envelope_drag(state, index, &mut editor);
        reset_envelope(state, index, None);
        editor.selected = None;
    } else if let Some(stage) = reset_stage {
        finish_envelope_drag(state, index, &mut editor);
        reset_envelope(state, index, Some(stage));
        editor.selected = Some(stage);
    } else if response.double_clicked()
        && let Some(stage) = hovered
    {
        finish_envelope_drag(state, index, &mut editor);
        reset_envelope(state, index, Some(stage));
        editor.selected = Some(stage);
    } else if response.drag_started()
        && let Some(stage) = hovered
    {
        begin_envelope_edit(state, index, stage);
        editor.selected = Some(stage);
        editor.drag = Some(stage);
        editor.drag_pointer_origin = response.interact_pointer_pos();
        editor.drag_handle_origin = handles
            .iter()
            .find_map(|(candidate, position)| (*candidate == stage).then_some(*position));
        editor.drag_precision = if ui.input(|input| input.modifiers.shift) {
            0.18
        } else {
            1.0
        };
    }
    if response.clicked() {
        editor.selected = hovered;
    }
    let drag_aborted =
        editor.drag.is_some() && ui.input(|input| !input.focused || !input.pointer.primary_down());
    if !drag_aborted
        && response.dragged()
        && let Some(stage) = editor.drag
    {
        let delta = ui.input(|input| input.pointer.delta());
        let requested_precision = if ui.input(|input| input.modifiers.shift) {
            0.18
        } else {
            1.0
        };
        let pointer = response.interact_pointer_pos();
        if matches!(
            stage,
            EnvelopeDrag::Attack | EnvelopeDrag::DecaySustain | EnvelopeDrag::Release
        ) && (editor.drag_precision - requested_precision).abs() > f32::EPSILON
        {
            editor.drag_pointer_origin = pointer;
            editor.drag_handle_origin = handles
                .iter()
                .find_map(|(candidate, position)| (*candidate == stage).then_some(*position));
            editor.drag_precision = requested_precision;
        }
        let y = delta.y / plot.height().max(1.0) * requested_precision;
        let dragged_x = |pointer: egui::Pos2, fallback: f32| {
            editor
                .drag_pointer_origin
                .zip(editor.drag_handle_origin)
                .map(|(pointer_origin, handle_origin)| {
                    handle_origin.x + (pointer.x - pointer_origin.x) * editor.drag_precision
                })
                .unwrap_or(fallback)
        };
        match stage {
            EnvelopeDrag::Attack => {
                if let Some(pointer) = pointer {
                    let x = dragged_x(pointer, points[1].x);
                    let seconds =
                        envelope_time_at_x(EnvelopeDrag::Attack, x, plot, attack, decay, release);
                    set_envelope_time(state, index, EnvelopeDrag::Attack, seconds);
                }
            }
            EnvelopeDrag::AttackCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::AttackCurve,
                    envelope_normalized(state, index, EnvelopeDrag::AttackCurve) - y,
                );
            }
            EnvelopeDrag::DecaySustain => {
                if let Some(pointer) = pointer {
                    let x = dragged_x(pointer, points[2].x);
                    let seconds = envelope_time_at_x(
                        EnvelopeDrag::DecaySustain,
                        x,
                        plot,
                        attack,
                        decay,
                        release,
                    );
                    set_envelope_time(state, index, EnvelopeDrag::DecaySustain, seconds);
                }
                set_envelope_sustain_normalized(
                    state,
                    index,
                    envelope_sustain_normalized(state, index) - y,
                );
            }
            EnvelopeDrag::DecayCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::DecayCurve,
                    envelope_normalized(state, index, EnvelopeDrag::DecayCurve) + y,
                );
            }
            EnvelopeDrag::Sustain => {
                set_envelope_sustain_normalized(
                    state,
                    index,
                    envelope_sustain_normalized(state, index) - y,
                );
            }
            EnvelopeDrag::Release => {
                if let Some(pointer) = pointer {
                    let x = dragged_x(pointer, points[3].x);
                    let seconds =
                        envelope_time_at_x(EnvelopeDrag::Release, x, plot, attack, decay, release);
                    set_envelope_time(state, index, EnvelopeDrag::Release, seconds);
                }
            }
            EnvelopeDrag::ReleaseCurve => {
                set_envelope_normalized(
                    state,
                    index,
                    EnvelopeDrag::ReleaseCurve,
                    envelope_normalized(state, index, EnvelopeDrag::ReleaseCurve) + y,
                );
            }
        }
        editor_theme::request_display_repaint(ui);
    }
    if response.drag_stopped() || drag_aborted {
        finish_envelope_drag(state, index, &mut editor);
    }

    let color = source_color(index);
    let curve_points = envelope_path(&points, curves);
    editor_widgets::gradient_area_to_baseline(&painter, &curve_points, plot.bottom(), color, 64);
    if response.hovered() || editor.drag.is_some() || editor.selected.is_some() {
        painter.rect_stroke(
            rect.shrink(editor_theme::shape::STROKE),
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                color.gamma_multiply(if editor.drag.is_some() { 0.7 } else { 0.42 }),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(stage) = editor.drag.or(hovered).or(editor.selected) {
        let (start, end) = envelope_segment(&points, stage);
        painter.add(egui::Shape::line(
            envelope_stage_path(start, end, envelope_curve_for_stage(curves, stage)),
            egui::Stroke::new(
                (plot.height() * 0.05).clamp(3.0, 5.0),
                color.gamma_multiply(if editor.drag == Some(stage) {
                    0.28
                } else {
                    0.14
                }),
            ),
        ));
    }
    painter.add(egui::Shape::line(
        curve_points,
        egui::Stroke::new((plot.height() * 0.014).clamp(1.25, 2.0), color),
    ));
    for (stage, position) in handles {
        let active = editor.drag == Some(stage);
        let hot = active || hovered == Some(stage);
        let selected = editor.selected == Some(stage);
        let curve_handle = matches!(
            stage,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve
        );
        let handle_radius = handle_radius * if curve_handle { 0.78 } else { 1.0 };
        if hot {
            painter.circle_filled(position, handle_radius * 1.55, color.gamma_multiply(0.18));
        }
        if selected {
            painter.circle_stroke(
                position,
                handle_radius * 1.25,
                egui::Stroke::new(
                    (handle_radius * 0.22).clamp(0.9, 1.35),
                    color.gamma_multiply(0.58),
                ),
            );
        }
        painter.circle_filled(
            position,
            if active {
                handle_radius * 1.08
            } else if hot || selected {
                handle_radius * 0.86
            } else {
                handle_radius * 0.68
            },
            if active {
                editor_theme::semantic().text
            } else {
                editor_theme::semantic().well
            },
        );
        painter.circle_stroke(
            position,
            if active {
                handle_radius * 1.08
            } else if hot || selected {
                handle_radius * 0.86
            } else {
                handle_radius * 0.68
            },
            egui::Stroke::new(
                (handle_radius * 0.2).clamp(0.8, 1.2),
                if active {
                    editor_theme::semantic().text
                } else {
                    color
                },
            ),
        );
        if hot || selected {
            let label = match stage {
                EnvelopeDrag::Attack => "A",
                EnvelopeDrag::AttackCurve => "A CURVE",
                EnvelopeDrag::DecaySustain => "D/S",
                EnvelopeDrag::DecayCurve => "D CURVE",
                EnvelopeDrag::Sustain => "S",
                EnvelopeDrag::Release => "R",
                EnvelopeDrag::ReleaseCurve => "R CURVE",
            };
            let label_y = if position.y - plot.top()
                < editor_theme::font::CAPTION_SIZE + handle_radius * 2.0
            {
                position.y + handle_radius * 1.4
            } else {
                position.y - handle_radius * 1.4
            };
            painter.text(
                egui::pos2(position.x, label_y),
                if label_y > position.y {
                    egui::Align2::CENTER_TOP
                } else {
                    egui::Align2::CENTER_BOTTOM
                },
                label,
                editor_theme::font::caption(),
                if active {
                    editor_theme::semantic().text
                } else {
                    color
                },
            );
        }
    }
    if response.hovered() {
        let hint = match editor.drag.or(hovered).or(editor.selected) {
            Some(EnvelopeDrag::Attack) => "ATTACK · DRAG X",
            Some(EnvelopeDrag::AttackCurve) => "ATTACK CURVE · DRAG Y",
            Some(EnvelopeDrag::DecaySustain) => "DECAY / SUSTAIN · DRAG X/Y",
            Some(EnvelopeDrag::DecayCurve) => "DECAY CURVE · DRAG Y",
            Some(EnvelopeDrag::Sustain) => "SUSTAIN · DRAG Y",
            Some(EnvelopeDrag::Release) => "RELEASE · DRAG X",
            Some(EnvelopeDrag::ReleaseCurve) => "RELEASE CURVE · DRAG Y",
            None => "DRAG A STAGE",
        };
        painter.text(
            plot.right_top() + egui::vec2(-editor_theme::space::XS, editor_theme::space::XXS),
            egui::Align2::RIGHT_TOP,
            hint,
            editor_theme::font::caption(),
            color.gamma_multiply(0.78),
        );
        ui.output_mut(|output| {
            output.cursor_icon = match editor.drag.or(hovered) {
                Some(_) if editor.drag.is_some() => egui::CursorIcon::Grabbing,
                Some(EnvelopeDrag::Attack | EnvelopeDrag::Release) => {
                    egui::CursorIcon::ResizeHorizontal
                }
                Some(EnvelopeDrag::DecaySustain) => egui::CursorIcon::ResizeNwSe,
                Some(
                    EnvelopeDrag::AttackCurve
                    | EnvelopeDrag::DecayCurve
                    | EnvelopeDrag::Sustain
                    | EnvelopeDrag::ReleaseCurve,
                ) => egui::CursorIcon::ResizeVertical,
                None => egui::CursorIcon::Default,
            };
        });
    }
    response.clone().on_hover_text(
        "Drag ADSR points or segments; drag midpoint handles vertically to bend stages. Hold Shift for fine adjustment. Double-click a stage or bend to reset it; right-click to reset the envelope.",
    );
    let value = source_value_meter(state, index).clamp(0.0, 1.0);
    painter.circle_filled(
        egui::pos2(plot.right(), egui::lerp(plot.bottom()..=plot.top(), value)),
        (plot.height() * 0.025).max(2.0),
        color,
    );
    let meter_moving = meter_is_moving(
        &mut editor.last_meter,
        &mut editor.meter_motion_frames,
        value,
        false,
    );
    request_graph_repaint(ui, meter_moving);
    ui.data_mut(|store| store.insert_temp(editor_id, editor));
}

fn envelope_stage_label(stage: EnvelopeDrag) -> &'static str {
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

fn envelope_segment(points: &[egui::Pos2; 5], stage: EnvelopeDrag) -> (egui::Pos2, egui::Pos2) {
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

fn envelope_time_at_x(
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

fn set_envelope_time(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
    seconds: f32,
) {
    let (maximum, param) = match stage {
        EnvelopeDrag::Attack => (8.0, Some(envelope_params(index).attack)),
        EnvelopeDrag::DecaySustain => (8.0, Some(envelope_params(index).decay)),
        EnvelopeDrag::Release => (12.0, Some(envelope_params(index).release)),
        _ => return,
    };
    let plain_fraction = (seconds / maximum).clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        state.set_param(
            param.expect("legacy envelope time has a host parameter"),
            f64::from(plain_fraction.powf(0.25)),
        );
    } else {
        set_envelope_normalized(state, index, stage, plain_fraction);
    }
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

fn envelope_stage_path(start: egui::Pos2, end: egui::Pos2, curve: f32) -> Vec<egui::Pos2> {
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

fn envelope_curve_for_stage(curves: [f32; 3], stage: EnvelopeDrag) -> f32 {
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

fn begin_envelope_edit(state: &PluginContext<KurvParams>, index: usize, stage: EnvelopeDrag) {
    if index >= LEGACY_MODULATION_SOURCES {
        return;
    }
    let params = envelope_params(index);
    match stage {
        EnvelopeDrag::Attack => state.begin_edit(params.attack),
        EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {}
        EnvelopeDrag::DecaySustain => {
            state.begin_edit(params.decay);
            state.begin_edit(params.sustain);
        }
        EnvelopeDrag::Sustain => state.begin_edit(params.sustain),
        EnvelopeDrag::Release => state.begin_edit(params.release),
    }
}

fn end_envelope_edit(state: &PluginContext<KurvParams>, index: usize, stage: EnvelopeDrag) {
    if index >= LEGACY_MODULATION_SOURCES {
        return;
    }
    let params = envelope_params(index);
    match stage {
        EnvelopeDrag::Attack => state.end_edit(params.attack),
        EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {}
        EnvelopeDrag::DecaySustain => {
            state.end_edit(params.decay);
            state.end_edit(params.sustain);
        }
        EnvelopeDrag::Sustain => state.end_edit(params.sustain),
        EnvelopeDrag::Release => state.end_edit(params.release),
    }
}

fn finish_envelope_drag(
    state: &PluginContext<KurvParams>,
    index: usize,
    editor: &mut EnvelopeEditorUi,
) {
    if let Some(stage) = editor.drag.take() {
        end_envelope_edit(state, index, stage);
    }
    editor.drag_pointer_origin = None;
    editor.drag_handle_origin = None;
    editor.drag_precision = 0.0;
}

fn reset_envelope(state: &PluginContext<KurvParams>, index: usize, stage: Option<EnvelopeDrag>) {
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let targets: &[P] = match stage {
            Some(EnvelopeDrag::Attack) => &[params.attack],
            Some(
                EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve,
            ) => &[],
            Some(EnvelopeDrag::DecaySustain) => &[params.decay, params.sustain],
            Some(EnvelopeDrag::Sustain) => &[params.sustain],
            Some(EnvelopeDrag::Release) => &[params.release],
            None => &[params.attack, params.decay, params.sustain, params.release],
        };
        for &param in targets {
            let raw = u32::from(param);
            let Some(default) = state
                .params()
                .param_infos()
                .into_iter()
                .find(|info| info.id == raw)
                .map(|info| info.range.normalize(info.default_plain))
            else {
                continue;
            };
            state.begin_edit(param);
            state.set_param(param, default);
            state.end_edit(param);
        }
        let defaults = crate::modulators::state::SourceConfig::default();
        let mut config = state.params().modulator_rack.config(index);
        match stage {
            Some(EnvelopeDrag::AttackCurve) => config.attack_curve = defaults.attack_curve,
            Some(EnvelopeDrag::DecayCurve) => config.decay_curve = defaults.decay_curve,
            Some(EnvelopeDrag::ReleaseCurve) => config.release_curve = defaults.release_curve,
            None => {
                config.attack_curve = defaults.attack_curve;
                config.decay_curve = defaults.decay_curve;
                config.release_curve = defaults.release_curve;
            }
            _ => {}
        }
        state.params().modulator_rack.set_config(index, config);
        return;
    }

    let defaults = crate::modulators::state::SourceConfig::default();
    let mut config = state.params().modulator_rack.config(index);
    match stage {
        Some(EnvelopeDrag::Attack) => config.attack = defaults.attack,
        Some(EnvelopeDrag::AttackCurve) => config.attack_curve = defaults.attack_curve,
        Some(EnvelopeDrag::DecaySustain) => {
            config.decay = defaults.decay;
            config.sustain = defaults.sustain;
        }
        Some(EnvelopeDrag::DecayCurve) => config.decay_curve = defaults.decay_curve,
        Some(EnvelopeDrag::Sustain) => config.sustain = defaults.sustain,
        Some(EnvelopeDrag::Release) => config.release = defaults.release,
        Some(EnvelopeDrag::ReleaseCurve) => config.release_curve = defaults.release_curve,
        None => {
            config.attack = defaults.attack;
            config.attack_curve = defaults.attack_curve;
            config.decay = defaults.decay;
            config.decay_curve = defaults.decay_curve;
            config.sustain = defaults.sustain;
            config.release = defaults.release;
            config.release_curve = defaults.release_curve;
        }
    }
    state.params().modulator_rack.set_config(index, config);
}

fn envelope_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
) -> f32 {
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let param = match stage {
            EnvelopeDrag::Attack => params.attack,
            EnvelopeDrag::AttackCurve => {
                return state
                    .params()
                    .modulator_rack
                    .config(index)
                    .attack_curve
                    .mul_add(0.5, 0.5);
            }
            EnvelopeDrag::DecaySustain => params.decay,
            EnvelopeDrag::DecayCurve => {
                return state
                    .params()
                    .modulator_rack
                    .config(index)
                    .decay_curve
                    .mul_add(0.5, 0.5);
            }
            EnvelopeDrag::Sustain => params.sustain,
            EnvelopeDrag::Release => params.release,
            EnvelopeDrag::ReleaseCurve => {
                return state
                    .params()
                    .modulator_rack
                    .config(index)
                    .release_curve
                    .mul_add(0.5, 0.5);
            }
        };
        return state.get_param(param);
    }
    let config = state.params().modulator_rack.config(index);
    match stage {
        EnvelopeDrag::Attack => config.attack / 8.0,
        EnvelopeDrag::AttackCurve => config.attack_curve.mul_add(0.5, 0.5),
        EnvelopeDrag::DecaySustain => config.decay / 8.0,
        EnvelopeDrag::DecayCurve => config.decay_curve.mul_add(0.5, 0.5),
        EnvelopeDrag::Sustain => config.sustain,
        EnvelopeDrag::Release => config.release / 12.0,
        EnvelopeDrag::ReleaseCurve => config.release_curve.mul_add(0.5, 0.5),
    }
}

fn envelope_sustain_normalized(state: &PluginContext<KurvParams>, index: usize) -> f32 {
    if index < LEGACY_MODULATION_SOURCES {
        state.get_param(envelope_params(index).sustain)
    } else {
        state.params().modulator_rack.config(index).sustain
    }
}

fn set_envelope_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    stage: EnvelopeDrag,
    normalized: f32,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        let params = envelope_params(index);
        let param = match stage {
            EnvelopeDrag::Attack => params.attack,
            EnvelopeDrag::DecaySustain => params.decay,
            EnvelopeDrag::Sustain => params.sustain,
            EnvelopeDrag::Release => params.release,
            EnvelopeDrag::AttackCurve | EnvelopeDrag::DecayCurve | EnvelopeDrag::ReleaseCurve => {
                let mut config = state.params().modulator_rack.config(index);
                let curve = normalized.mul_add(2.0, -1.0);
                match stage {
                    EnvelopeDrag::AttackCurve => config.attack_curve = curve,
                    EnvelopeDrag::DecayCurve => config.decay_curve = curve,
                    EnvelopeDrag::ReleaseCurve => config.release_curve = curve,
                    _ => unreachable!(),
                }
                state.params().modulator_rack.set_config(index, config);
                return;
            }
        };
        state.set_param(param, f64::from(normalized));
        return;
    }
    let mut config = state.params().modulator_rack.config(index);
    match stage {
        EnvelopeDrag::Attack => config.attack = normalized * 8.0,
        EnvelopeDrag::AttackCurve => config.attack_curve = normalized.mul_add(2.0, -1.0),
        EnvelopeDrag::DecaySustain => config.decay = normalized * 8.0,
        EnvelopeDrag::DecayCurve => config.decay_curve = normalized.mul_add(2.0, -1.0),
        EnvelopeDrag::Sustain => config.sustain = normalized,
        EnvelopeDrag::Release => config.release = normalized * 12.0,
        EnvelopeDrag::ReleaseCurve => config.release_curve = normalized.mul_add(2.0, -1.0),
    }
    state.params().modulator_rack.set_config(index, config);
}

fn set_envelope_sustain_normalized(
    state: &PluginContext<KurvParams>,
    index: usize,
    normalized: f32,
) {
    let normalized = normalized.clamp(0.0, 1.0);
    if index < LEGACY_MODULATION_SOURCES {
        state.set_param(envelope_params(index).sustain, f64::from(normalized));
        return;
    }
    let mut config = state.params().modulator_rack.config(index);
    config.sustain = normalized;
    state.params().modulator_rack.set_config(index, config);
}
