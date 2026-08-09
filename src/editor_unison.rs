//! Unison distribution, stereo blend, and direct point-shaper views.

use crate::pan_curve::{
    PanShapeCurveData, PanShapeCurveState, PanShapeKnot, insert_knot, move_center, move_endpoint,
    move_knot, remove_knot, set_segment_curve,
};
use crate::voices::PanShapeSettings;
use crate::voices::{
    JITTER_EXCURSION_CENTS, MAX_UNISON, SwarmMode, UnisonAlignmentMode, UnisonSettings,
    fill_oscillator_unison_layout, fill_unison_jitter_offsets_mode, unison_static_pitch_cents,
};
use crate::{editor_envelope, editor_theme, editor_widgets};

const CURVE_POINTS: u16 = 96;

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

fn custom_pan_shape_curve_view(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    id: egui::Id,
    curve_state: &PanShapeCurveState,
    center_x: &mut f32,
) -> bool {
    let response = ui
        .interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Crosshair)
        .on_hover_text("Drag the curve points; double-click to add and right-click to remove");
    let plot = rect.shrink2(egui::vec2(5.0, 3.0));
    let pointer = response.interact_pointer_pos();
    let drag_id = id.with("point-drag");
    let mut data = curve_state.snapshot();
    let mut active = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id));
    let mut changed = false;

    if active.is_none()
        && response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer.filter(|pointer| plot.contains(*pointer))
        && !pan_shape_hit_any(&data, plot, *center_x, pointer)
    {
        let (left, input, output) = pan_shape_values_from_pos(plot, *center_x, pointer);
        let mirror = ui.input(|input| input.modifiers.shift);
        changed = curve_state.edit(|curve| {
            let mut candidate = curve.clone();
            if !insert_knot(candidate.half_mut(left), input, output)
                || (mirror && !insert_knot(candidate.half_mut(!left), input, output))
            {
                return false;
            }
            *curve = candidate;
            true
        });
        if changed {
            data = curve_state.snapshot();
        }
    }

    if active.is_none()
        && response.clicked_by(egui::PointerButton::Secondary)
        && let Some(pointer) = pointer
        && let Some((left, index)) = pan_shape_hit_knot(&data, plot, *center_x, pointer)
    {
        let mirror = ui.input(|input| input.modifiers.shift);
        let mirror_index = mirror
            .then(|| matching_knot_index(data.half(!left), data.half(left).knots[index].in_lin));
        changed |= curve_state.edit(|curve| {
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
        if changed {
            data = curve_state.snapshot();
        }
    }

    if active.is_none()
        && response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer
    {
        let target = if pan_shape_hit_center(&data, plot, *center_x, pointer) {
            Some(PanShapePointDragTarget::Center)
        } else if pointer.distance(pan_shape_endpoint(&data, plot, *center_x, true)) <= 12.0 {
            Some(PanShapePointDragTarget::Endpoint { left: true })
        } else if pointer.distance(pan_shape_endpoint(&data, plot, *center_x, false)) <= 12.0 {
            Some(PanShapePointDragTarget::Endpoint { left: false })
        } else if let Some((left, index)) = pan_shape_hit_knot(&data, plot, *center_x, pointer) {
            Some(PanShapePointDragTarget::Knot { left, index })
        } else if let Some((left, index)) = pan_shape_hit_curve(&data, plot, *center_x, pointer) {
            Some(PanShapePointDragTarget::Curve { left, index })
        } else {
            None
        };
        if let Some(target) = target {
            let drag = PanShapePointDrag {
                target,
                anchor: pan_shape_target_pos(&data, plot, *center_x, target),
            };
            ui.data_mut(|store| store.insert_temp(drag_id, drag.clone()));
            active = Some(drag);
        }
    }

    if let Some(drag) = active.as_ref()
        && response.dragged_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer
    {
        let (pointer, mirror) = ui.input(|input| {
            (
                constrain_drag(
                    drag.anchor,
                    pointer,
                    input.modifiers.alt
                        && !matches!(drag.target, PanShapePointDragTarget::Endpoint { .. }),
                ),
                input.modifiers.shift,
            )
        });
        let target = drag.target;
        curve_state.edit(|curve| match target {
            PanShapePointDragTarget::Center => {
                let (_, _, output) = pan_shape_values_from_pos(plot, *center_x, pointer);
                let normalized_x =
                    ((pointer.x - plot.left()) / plot.width().max(1.0)).clamp(0.0, 1.0);
                move_center(curve, output);
                *center_x = normalized_x.mul_add(0.9, 0.05);
            }
            PanShapePointDragTarget::Endpoint { left } => {
                let (_, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                move_endpoint(curve.half_mut(left), output);
                if mirror {
                    move_endpoint(curve.half_mut(!left), output);
                }
            }
            PanShapePointDragTarget::Knot { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                let mirror_index = mirror.then(|| {
                    matching_knot_index(curve.half(!left), curve.half(left).knots[index].in_lin)
                });
                move_knot(curve.half_mut(left), index, input, output);
                if let Some(Some(index)) = mirror_index {
                    move_knot(curve.half_mut(!left), index, input, output);
                }
            }
            PanShapePointDragTarget::Curve { left, index } => {
                let (input, output) = pan_shape_values_from_side(plot, *center_x, left, pointer);
                let mirror_index = mirror.then(|| {
                    let half = curve.half(left);
                    matching_segment_index(
                        curve.half(!left),
                        (half.knots[index].in_lin + half.knots[index + 1].in_lin) * 0.5,
                    )
                });
                let half = curve.half_mut(left);
                let start = half.knots[index].out_lin;
                let end = half.knots[index + 1].out_lin;
                let segment_start = half.knots[index].in_lin;
                let segment_end = half.knots[index + 1].in_lin;
                let vertical = if (end - start).abs() > f32::EPSILON {
                    ((output - start) / (end - start)).clamp(0.0, 1.0)
                } else {
                    0.5
                }
                .mul_add(2.0, -1.0);
                let horizontal = ((((input - segment_start)
                    / (segment_end - segment_start).max(f32::EPSILON))
                .clamp(0.0, 1.0)
                    - 0.5)
                    / 0.44)
                    .clamp(-1.0, 1.0);
                set_segment_curve(half, index, vertical, horizontal);
                if let Some(Some(index)) = mirror_index {
                    set_segment_curve(curve.half_mut(!left), index, vertical, horizontal);
                }
            }
        });
        data = curve_state.snapshot();
        changed = true;
        editor_theme::request_display_repaint(ui);
    }

    if response.drag_stopped_by(egui::PointerButton::Primary) {
        ui.data_mut(|store| store.remove::<PanShapePointDrag>(drag_id));
        active = None;
    }
    draw_pan_shape_curve(
        painter, rect, plot, *center_x, &data, pointer, active, false,
    );
    changed
}

pub(crate) fn custom_unison_view(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    config: &mut crate::generators::OscillatorConfig,
    pan_shape_curve: &PanShapeCurveState,
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
        config.unison_pan_center_x.to_bits(),
        config.unison_stereo_x.to_bits(),
        config.unison_stereo_alternate.to_bits(),
    );
    painter.rect_filled(outer.rect, 0.0, editor_theme::semantic().well);
    let view_id = outer.id.with("view");
    let current_view = ui
        .data(|data| data.get_temp::<CompactUnisonView>(view_id))
        .unwrap_or_default();
    let selected_view = compact_unison_view_tabs(ui, &painter, header, view_id, current_view);
    let mut pan_curve_changed = false;
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
            if !pan_shape_curve.is_initialized() {
                pan_shape_curve.replace(PanShapeCurveData::from_legacy(
                    0.0,
                    1.0,
                    1.0,
                    config.unison_pan_curve,
                    config.unison_pan_curve,
                    0.5,
                    0.5,
                ));
            }
            pan_curve_changed |= custom_pan_shape_curve_view(
                ui,
                &painter,
                pan_shape_rect,
                outer.id.with("pan-shape"),
                pan_shape_curve,
                &mut config.unison_pan_center_x,
            );
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
    fill_unison_jitter_offsets_mode(
        &mut jitter_offsets[..voices],
        0.618_034,
        config.unison_jitter,
        time,
        SwarmMode::from_index(config.unison_jitter_mode),
    );
    let pan_shape = PanShapeSettings::default()
        .with_center_x(config.unison_pan_center_x)
        .with_curve_data(&pan_shape_curve.snapshot());
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
    fill_oscillator_unison_layout(
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
            config.unison_pan_center_x.to_bits(),
            config.unison_stereo_x.to_bits(),
            config.unison_stereo_alternate.to_bits(),
        )
        || pan_curve_changed
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
