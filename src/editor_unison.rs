//! Unison distribution, stereo blend, and direct point-shaper views.

use truce_core::editor::PluginContext;

use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::pan_curve::{
    PanShapeCurveData, PanShapeCurveState, PanShapeKnot, insert_knot, move_center, move_endpoint,
    move_knot, remove_knot, set_segment_curve,
};
use crate::voices::PanShapeSettings;
use crate::voices::{
    JITTER_EXCURSION_CENTS, MAX_UNISON, SwarmMode, UnisonAlignmentMode, UnisonSettings,
    fill_oscillator_unison_layout, fill_unison_jitter_offsets_mode, unison_static_pitch_cents,
};
use crate::{KurvParams, editor_theme, editor_widgets};

const CURVE_POINTS: u16 = 96;

#[derive(Clone, PartialEq)]
struct CompactUnisonPreviewKey {
    plot: egui::Rect,
    voices: u8,
    range: u32,
    amount: u32,
    curve: u32,
    width: u32,
    phase_random: u32,
    alignment: u32,
    alignment_mode: u8,
    pan_center_x: u32,
    stereo_x: u32,
    stereo_alternate: u32,
    pan_segments: (
        crate::pan_curve::PanShapeSegmentsRt,
        crate::pan_curve::PanShapeSegmentsRt,
    ),
}

#[derive(Clone)]
struct CompactUnisonPreview {
    key: CompactUnisonPreviewKey,
    points: [egui::Pos2; MAX_UNISON],
}

#[derive(Clone)]
struct PanShapePointDrag {
    target: PanShapePointDragTarget,
    anchor: egui::Pos2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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

fn compact_unison_layout(rect: egui::Rect) -> (egui::Rect, egui::Rect, egui::Rect, egui::Rect) {
    let inset = editor_theme::space::XXS.min(rect.width().min(rect.height()) * 0.035);
    let content = rect.shrink(inset);
    let header_height =
        (editor_theme::font::LABEL_SIZE + editor_theme::space::XXS * 2.0).min(content.height());
    let header = egui::Rect::from_min_size(content.min, egui::vec2(content.width(), header_height));
    let view = content;
    let rail_width =
        (editor_theme::font::CAPTION_SIZE + editor_theme::space::XS).min(content.width());
    let rail = egui::Rect::from_min_max(
        egui::pos2(
            (view.right() - rail_width).max(view.left()),
            header.bottom(),
        ),
        view.max,
    );
    let plot = egui::Rect::from_min_max(
        egui::pos2(view.left(), header.bottom()),
        egui::pos2(
            (rail.left() - editor_theme::space::XXS).max(view.left()),
            view.bottom(),
        ),
    );
    (header, view, plot, rail)
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
    child.spacing_mut().button_padding = egui::vec2(rect.height() * 0.22, rect.height() * 0.05);
    let palette = editor_theme::semantic();
    let visuals = child.visuals_mut();
    visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.bg_fill = plugcat::theme::mix(palette.well, palette.unison, 0.12);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.bg_fill = plugcat::theme::mix(palette.well, palette.unison, 0.20);
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    let response = egui::ComboBox::from_id_salt(id.with("menu"))
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
    response
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Unison alignment mode");
    selected
}

pub(crate) fn vertical_selector_value(rect: egui::Rect, pointer: egui::Pos2) -> f32 {
    ((rect.bottom() - pointer.y) / rect.height().max(f32::EPSILON)).clamp(0.0, 1.0)
}

pub(crate) fn paint_vertical_selector(
    painter: &egui::Painter,
    rect: egui::Rect,
    value: f32,
    color: egui::Color32,
) {
    paint_vertical_selector_state(painter, rect, value, color, false, false);
}

fn paint_vertical_selector_state(
    painter: &egui::Painter,
    rect: egui::Rect,
    value: f32,
    color: egui::Color32,
    hovered: bool,
    active: bool,
) {
    let track_x = rect.center().x;
    let rail_inset =
        (editor_theme::font::CAPTION_SIZE + editor_theme::space::XXS).min(rect.height() * 0.18);
    let top = rect.top() + rail_inset;
    let bottom = rect.bottom() - rail_inset;
    let y = egui::lerp(bottom..=top, value.clamp(0.0, 1.0));
    let base_stroke = editor_theme::shape::STROKE;
    painter.line_segment(
        [egui::pos2(track_x, top), egui::pos2(track_x, bottom)],
        egui::Stroke::new(
            base_stroke,
            color.gamma_multiply(if active {
                0.52
            } else if hovered {
                0.36
            } else {
                0.20
            }),
        ),
    );
    painter.line_segment(
        [egui::pos2(track_x, y), egui::pos2(track_x, bottom)],
        egui::Stroke::new(
            base_stroke * if active { 1.75 } else { 1.4 },
            color.gamma_multiply(if active {
                1.0
            } else if hovered {
                0.90
            } else {
                0.78
            }),
        ),
    );
    let thumb_radius = editor_theme::font::CAPTION_SIZE
        * if active {
            0.48
        } else if hovered {
            0.42
        } else {
            0.34
        };
    let thumb = egui::pos2(track_x, y);
    painter.circle_filled(thumb, thumb_radius, color);
    if hovered || active {
        painter.circle_stroke(
            thumb,
            thumb_radius * 1.55,
            egui::Stroke::new(
                editor_theme::shape::STROKE,
                color.gamma_multiply(if active { 0.72 } else { 0.42 }),
            ),
        );
    }
}

fn custom_pan_shape_curve_view(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    id: egui::Id,
    curve_state: &PanShapeCurveState,
    center_x: &mut f32,
) -> (bool, egui::Response) {
    let plot = rect.shrink2(egui::vec2(
        editor_theme::space::XXS.min(rect.width() * 0.035),
        editor_theme::space::XXS.min(rect.height() * 0.04),
    ));
    let response = ui.interact(plot, id, egui::Sense::click_and_drag());
    response
        .clone()
        .on_hover_text("Drag points or curve handles; double-click to add, right-click to remove");
    let hit_radius = editor_theme::title_height(ui) * 0.52;
    let handle_radius = editor_theme::font::CAPTION_SIZE * 0.38;
    let pointer = response.interact_pointer_pos();
    let drag_id = id.with("point-drag");
    let mut data = curve_state.snapshot();
    let mut active = ui.data(|store| store.get_temp::<PanShapePointDrag>(drag_id));
    let mut hovered_target = pointer
        .and_then(|pointer| pan_shape_hit_target(&data, plot, *center_x, pointer, hit_radius));
    let mut changed = false;

    if active.is_none()
        && response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pointer) = pointer.filter(|pointer| plot.contains(*pointer))
        && hovered_target.is_none()
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
        && let Some(PanShapePointDragTarget::Knot { left, index }) = hovered_target
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
        && let Some(target) = hovered_target
    {
        let drag = PanShapePointDrag {
            target,
            anchor: pan_shape_target_pos(&data, plot, *center_x, target),
        };
        ui.data_mut(|store| store.insert_temp(drag_id, drag.clone()));
        active = Some(drag);
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
    hovered_target = pointer
        .and_then(|pointer| pan_shape_hit_target(&data, plot, *center_x, pointer, hit_radius));
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = if active.is_some() {
                egui::CursorIcon::Grabbing
            } else if hovered_target.is_some() {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
    draw_pan_shape_curve(
        painter,
        plot,
        *center_x,
        &data,
        hovered_target,
        active,
        response.hovered(),
        handle_radius,
    );
    (changed, response)
}

fn compact_unison_preview_points(
    config: &crate::generators::OscillatorConfig,
    plot: egui::Rect,
    pan_segments: (
        crate::pan_curve::PanShapeSegmentsRt,
        crate::pan_curve::PanShapeSegmentsRt,
    ),
    time: f32,
) -> [egui::Pos2; MAX_UNISON] {
    let voices_u8 = config.unison_voices.clamp(1, MAX_UNISON as u8);
    let voices = usize::from(voices_u8);
    let full_scale =
        (config.unison_range * 100.0 + JITTER_EXCURSION_CENTS * config.unison_jitter).max(1.0);
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
        .with_segments(pan_segments);
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
    let alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
    let mut points = [egui::Pos2::ZERO; MAX_UNISON];
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
            ((detune + jitter) / full_scale).mul_add(plot.width() * 0.46, plot.center().x),
            (-pan).mul_add(plot.height() * 0.38, plot.center().y),
        );
    }
    points
}

fn host_axes_context_menu(
    response: &egui::Response,
    state: &PluginContext<KurvParams>,
    axes: &[(&str, ModulationRouteTarget, f32)],
) {
    response.context_menu(|ui| {
        ui.label(
            egui::RichText::new("HOST AUTOMATION")
                .font(editor_theme::font::caption())
                .color(editor_theme::semantic().text_muted),
        );
        for (label, target, base) in axes.iter().copied() {
            ui.menu_button(label, |ui| {
                crate::editor_modulation::host_automation_menu(ui, state, target, base);
            });
        }
    });
}

fn update_host_axis(
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    normalized: f32,
    changed: bool,
) {
    if let Some((_, param, _)) = crate::editor_modulation::host_automation_binding(state, target) {
        crate::editor_modulation::update_host_automation_gesture(
            state, param, response, normalized, changed,
        );
    }
}

pub(crate) fn custom_unison_distribution_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    width: f32,
    height: f32,
    config: &mut crate::generators::OscillatorConfig,
    pan_shape_curve: &PanShapeCurveState,
) -> bool {
    let (outer, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::hover(),
    );
    let (header, _, unison_plot, alignment_rail) = compact_unison_layout(outer.rect);
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
    let palette = editor_theme::semantic();
    painter.rect_filled(
        outer.rect,
        0.0,
        plugcat::theme::mix(palette.well, palette.chrome, 0.18),
    );
    let alignment_mode = UnisonAlignmentMode::from_index(config.unison_alignment_mode);
    let mode_width = (editor_theme::font::VALUE_SIZE * 6.0)
        .min((alignment_rail.left() - header.left()).max(1.0) * 0.42);
    let mode_rect = egui::Rect::from_min_max(
        egui::pos2(
            (alignment_rail.left() - editor_theme::space::XXS - mode_width).max(header.left()),
            header.top(),
        ),
        egui::pos2(
            (alignment_rail.left() - editor_theme::space::XXS).max(header.left()),
            header.bottom(),
        ),
    );
    if let Some(mode) = compact_alignment_mode_combo(
        ui,
        mode_rect,
        outer.id.with("alignment-mode"),
        alignment_mode,
    ) {
        config.unison_alignment_mode = mode.index();
    }

    let distribution_id = outer.id.with("distribution");
    let response = ui.interact(unison_plot, distribution_id, egui::Sense::click_and_drag());
    response
        .clone()
        .on_hover_text("Drag horizontally for detune; vertically for distribution curve");
    let distribution_hovered = response.hovered();
    let distribution_active = response.dragged() || response.is_pointer_button_down_on();
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = if distribution_active {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
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

    let alignment_response = ui
        .interact(
            alignment_rail,
            outer.id.with("alignment-amount"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Unison alignment amount");
    if (alignment_response.drag_started()
        || alignment_response.dragged()
        || alignment_response.clicked())
        && let Some(pointer) = alignment_response.interact_pointer_pos()
    {
        config.unison_alignment = vertical_selector_value(alignment_rail, pointer);
    }
    let amount_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::UnisonAmount);
    let curve_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::UnisonCurve);
    let alignment_target =
        ModulationRouteTarget::oscillator(module_id, slot, OscillatorControl::UnisonAlignment);
    let amount = OscillatorControl::UnisonAmount.normalized_value(*config);
    let curve = OscillatorControl::UnisonCurve.normalized_value(*config);
    let alignment = OscillatorControl::UnisonAlignment.normalized_value(*config);
    host_axes_context_menu(
        &response,
        state,
        &[
            ("X · DETUNE", amount_target, amount),
            ("Y · DISTRIBUTION", curve_target, curve),
        ],
    );
    crate::editor_modulation::host_automation_destination(
        ui,
        state,
        alignment_target,
        &alignment_response,
        alignment,
    );
    update_host_axis(
        state,
        amount_target,
        &response,
        amount,
        before.0 != config.unison_amount.to_bits(),
    );
    update_host_axis(
        state,
        curve_target,
        &response,
        curve,
        before.1 != config.unison_curve.to_bits(),
    );
    update_host_axis(
        state,
        alignment_target,
        &alignment_response,
        alignment,
        before.2 != config.unison_alignment.to_bits(),
    );
    paint_vertical_selector_state(
        &painter,
        alignment_rail,
        config.unison_alignment,
        palette.unison,
        alignment_response.hovered(),
        alignment_response.dragged() || alignment_response.is_pointer_button_down_on(),
    );

    let voices_u8 = config.unison_voices.clamp(1, MAX_UNISON as u8);
    let voices = usize::from(voices_u8);
    let jitter_active = config.unison_jitter > f32::EPSILON;
    let time = if jitter_active {
        ui.input(|input| input.time) as f32 * normalized_unison_rate(config.unison_rate)
    } else {
        0.0
    };
    if jitter_active {
        editor_theme::request_display_repaint(ui);
    }
    let weights = [1.0_f32; MAX_UNISON];
    let pan_segments = pan_shape_curve.segments_rt();
    let preview_key = CompactUnisonPreviewKey {
        plot: unison_plot,
        voices: voices_u8,
        range: config.unison_range.to_bits(),
        amount: config.unison_amount.to_bits(),
        curve: config.unison_curve.to_bits(),
        width: config.unison_width.to_bits(),
        phase_random: config.phase_random.to_bits(),
        alignment: config.unison_alignment.to_bits(),
        alignment_mode: config.unison_alignment_mode,
        pan_center_x: config.unison_pan_center_x.to_bits(),
        stereo_x: config.unison_stereo_x.to_bits(),
        stereo_alternate: config.unison_stereo_alternate.to_bits(),
        pan_segments,
    };
    let preview_id = outer.id.with("unison-preview");
    let cached = (!jitter_active)
        .then(|| ui.data(|data| data.get_temp::<CompactUnisonPreview>(preview_id)))
        .flatten()
        .filter(|preview| preview.key == preview_key);
    let points = cached.map_or_else(
        || {
            let points = compact_unison_preview_points(config, unison_plot, pan_segments, time);
            if !jitter_active {
                ui.data_mut(|data| {
                    data.insert_temp(
                        preview_id,
                        CompactUnisonPreview {
                            key: preview_key,
                            points,
                        },
                    );
                });
            }
            points
        },
        |preview| preview.points,
    );
    paint_compact_distribution(
        &painter,
        unison_plot,
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
        distribution_hovered,
        distribution_active,
    );
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
}

pub(crate) fn normalized_unison_rate(normalized: f32) -> f32 {
    0.02 * 5_000.0_f32.powf(normalized.clamp(0.0, 1.0))
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum CompactPanView {
    #[default]
    PanXy,
    PanShape,
}

pub(crate) fn custom_pan_panel_view(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    module_id: ModuleId,
    slot: OscillatorSlot,
    width: f32,
    height: f32,
    config: &mut crate::generators::OscillatorConfig,
    pan_shape_curve: &PanShapeCurveState,
) -> bool {
    let (outer, painter) = ui.allocate_painter(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::hover(),
    );
    let before = (
        config.unison_pan_center_x.to_bits(),
        config.unison_stereo_x.to_bits(),
        config.unison_stereo_alternate.to_bits(),
    );
    let palette = editor_theme::semantic();
    painter.rect_filled(
        outer.rect,
        0.0,
        plugcat::theme::mix(palette.well, palette.chrome, 0.18),
    );
    let frame = outer
        .rect
        .shrink(editor_theme::space::XXS.min(outer.rect.width().min(outer.rect.height()) * 0.025));
    let header_height = (editor_theme::font::LABEL_SIZE + editor_theme::space::XXS * 2.0)
        .min(frame.height() * 0.24);
    let header = egui::Rect::from_min_max(
        frame.min,
        egui::pos2(
            frame.right(),
            (frame.top() + header_height).min(frame.bottom()),
        ),
    );
    let content = egui::Rect::from_min_max(egui::pos2(frame.left(), header.bottom()), frame.max);
    let view_id = outer.id.with("pan-view");
    let current = ui
        .data(|data| data.get_temp::<CompactPanView>(view_id))
        .unwrap_or_default();
    let mut selected = current;
    let mut pan_shape_tab_response = None;
    let compact_tabs = header.width() * 0.5 < editor_theme::font::LABEL_SIZE * 6.5;
    for (view, label, compact_label, accent) in [
        (CompactPanView::PanXy, "PAN X/Y", "X/Y", palette.pan_shape),
        (
            CompactPanView::PanShape,
            "PAN SHAPE",
            "SHAPE",
            palette.pan_shape,
        ),
    ] {
        let tab = egui::Rect::from_min_max(
            egui::pos2(
                header.left()
                    + header.width()
                        * if matches!(view, CompactPanView::PanXy) {
                            0.0
                        } else {
                            0.5
                        },
                header.top(),
            ),
            egui::pos2(
                header.left()
                    + header.width()
                        * if matches!(view, CompactPanView::PanXy) {
                            0.5
                        } else {
                            1.0
                        },
                header.bottom(),
            ),
        );
        let response = ui
            .interact(tab, view_id.with(label), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if matches!(view, CompactPanView::PanShape) {
            pan_shape_tab_response = Some(response.clone());
        }
        if response.clicked() {
            selected = view;
        }
        let active = response.is_pointer_button_down_on();
        if selected == view || response.hovered() || active {
            painter.rect_filled(
                tab,
                0.0,
                plugcat::theme::mix(
                    palette.well,
                    accent,
                    if active {
                        0.12
                    } else if selected == view {
                        0.075
                    } else {
                        0.045
                    },
                ),
            );
        }
        painter.text(
            tab.center(),
            egui::Align2::CENTER_CENTER,
            if compact_tabs { compact_label } else { label },
            editor_theme::font::caption(),
            accent.gamma_multiply(if active {
                1.0
            } else if selected == view {
                1.0
            } else if response.hovered() {
                0.82
            } else {
                0.52
            }),
        );
        if selected == view {
            painter.line_segment(
                [tab.left_bottom(), tab.right_bottom()],
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, accent),
            );
        }
    }
    if selected != current {
        ui.data_mut(|data| data.insert_temp(view_id, selected));
    }

    let mut curve_changed = false;
    match selected {
        CompactPanView::PanXy => {
            let response = custom_stereo_square_view(
                ui,
                &painter,
                content,
                outer.id.with("stereo-square"),
                &mut config.unison_stereo_x,
                &mut config.unison_stereo_alternate,
            );
            let x_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoPosition,
            );
            let y_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoAlternate,
            );
            let x = OscillatorControl::UnisonStereoPosition.normalized_value(*config);
            let y = OscillatorControl::UnisonStereoAlternate.normalized_value(*config);
            host_axes_context_menu(
                &response,
                state,
                &[("X · PATTERN", x_target, x), ("Y · BLEND", y_target, y)],
            );
            update_host_axis(
                state,
                x_target,
                &response,
                x,
                before.1 != config.unison_stereo_x.to_bits(),
            );
            update_host_axis(
                state,
                y_target,
                &response,
                y,
                before.2 != config.unison_stereo_alternate.to_bits(),
            );
        }
        CompactPanView::PanShape => {
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
            let split_gap = editor_theme::compact_gap(ui);
            let available_width = (content.width() - split_gap).max(1.0);
            let curve_width = available_width * 0.5;
            let curve_rect = egui::Rect::from_min_size(
                content.min,
                egui::vec2(curve_width.max(1.0), content.height()),
            );
            let xy_rect = egui::Rect::from_min_max(
                egui::pos2(curve_rect.right() + split_gap, content.top()),
                content.max,
            );
            painter.line_segment(
                [
                    egui::pos2(curve_rect.right() + split_gap * 0.5, content.top()),
                    egui::pos2(curve_rect.right() + split_gap * 0.5, content.bottom()),
                ],
                egui::Stroke::new(
                    editor_theme::shape::STROKE,
                    palette.grid.gamma_multiply(0.34),
                ),
            );
            let (changed, curve_response) = custom_pan_shape_curve_view(
                ui,
                &painter,
                curve_rect,
                outer.id.with("pan-shape"),
                pan_shape_curve,
                &mut config.unison_pan_center_x,
            );
            curve_changed |= changed;
            let target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonPanCenter,
            );
            let normalized = OscillatorControl::UnisonPanCenter.normalized_value(*config);
            if let Some(tab) = pan_shape_tab_response.as_ref() {
                tab.context_menu(|ui| {
                    crate::editor_modulation::host_automation_menu(ui, state, target, normalized);
                });
            }
            update_host_axis(
                state,
                target,
                &curve_response,
                normalized,
                before.0 != config.unison_pan_center_x.to_bits(),
            );
            let xy_response = custom_stereo_square_view(
                ui,
                &painter,
                xy_rect,
                outer.id.with("pan-shape-stereo-square"),
                &mut config.unison_stereo_x,
                &mut config.unison_stereo_alternate,
            );
            let x_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoPosition,
            );
            let y_target = ModulationRouteTarget::oscillator(
                module_id,
                slot,
                OscillatorControl::UnisonStereoAlternate,
            );
            let x = OscillatorControl::UnisonStereoPosition.normalized_value(*config);
            let y = OscillatorControl::UnisonStereoAlternate.normalized_value(*config);
            host_axes_context_menu(
                &xy_response,
                state,
                &[("X · PATTERN", x_target, x), ("Y · BLEND", y_target, y)],
            );
            update_host_axis(
                state,
                x_target,
                &xy_response,
                x,
                before.1 != config.unison_stereo_x.to_bits(),
            );
            update_host_axis(
                state,
                y_target,
                &xy_response,
                y,
                before.2 != config.unison_stereo_alternate.to_bits(),
            );
        }
    }
    before
        != (
            config.unison_pan_center_x.to_bits(),
            config.unison_stereo_x.to_bits(),
            config.unison_stereo_alternate.to_bits(),
        )
        || curve_changed
}

fn paint_compact_distribution(
    painter: &egui::Painter,
    plot: egui::Rect,
    points: &[egui::Pos2],
    weights: &[f32],
    maximum_weight: f32,
    control_point: egui::Pos2,
    opacity: f32,
    hovered: bool,
    active: bool,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    let palette = editor_theme::semantic();
    if hovered || active {
        let guide = egui::Stroke::new(
            1.0_f32,
            palette
                .unison
                .linear_multiply(if active { 0.30 } else { 0.18 }),
        );
        painter.line_segment(
            [
                egui::pos2(plot.left(), control_point.y),
                egui::pos2(plot.right(), control_point.y),
            ],
            guide,
        );
        painter.line_segment(
            [
                egui::pos2(control_point.x, plot.top()),
                egui::pos2(control_point.x, plot.bottom()),
            ],
            guide,
        );
    }
    for (point, weight) in points.iter().zip(weights) {
        let relative = (weight / maximum_weight.max(f32::EPSILON)).sqrt();
        let half_height = plot.height() * relative.mul_add(0.055, 0.025);
        let color = palette
            .unison
            .linear_multiply(relative.mul_add(0.72, 0.28) * opacity);
        painter.line_segment(
            [
                *point - egui::vec2(0.0, half_height),
                *point + egui::vec2(0.0, half_height),
            ],
            egui::Stroke::new(editor_theme::font::CAPTION_SIZE * 0.20, color),
        );
    }
    let control_radius = editor_theme::font::CAPTION_SIZE
        * if active {
            0.48
        } else if hovered {
            0.42
        } else {
            0.35
        };
    painter.circle_filled(
        control_point,
        control_radius,
        palette.unison.linear_multiply(opacity),
    );
    if hovered || active {
        painter.circle_stroke(
            control_point,
            control_radius * 1.65,
            egui::Stroke::new(
                1.0_f32,
                palette
                    .unison
                    .linear_multiply(if active { opacity } else { opacity * 0.62 }),
            ),
        );
    }
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
        let x =
            ((point.x - self.rect.left()) / self.rect.width().max(f32::EPSILON)).clamp(0.0, 1.0);
        let y =
            ((self.rect.bottom() - point.y) / self.rect.height().max(f32::EPSILON)).clamp(0.0, 1.0);
        (x, y)
    }

    fn snap(self, axes: (f32, f32), enabled: bool, radius: f32) -> (f32, f32) {
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
            .filter(|candidate| self.point(candidate.0, candidate.1).distance(point) <= radius)
            .min_by(|left, right| {
                self.point(left.0, left.1)
                    .distance_sq(point)
                    .total_cmp(&self.point(right.0, right.1).distance_sq(point))
            })
            .unwrap_or(axes)
    }
}

fn stereo_square_plot(rect: egui::Rect) -> egui::Rect {
    rect.shrink(
        (editor_theme::font::CAPTION_SIZE * 0.54).min(rect.width().min(rect.height()) * 0.12),
    )
}

fn paint_stereo_square(
    painter: &egui::Painter,
    plot: egui::Rect,
    x: f32,
    y: f32,
    hovered: bool,
    active: bool,
) {
    let palette = editor_theme::semantic();
    let accent = palette.pan_shape;
    let emphasis = if active {
        1.0
    } else if hovered {
        0.78
    } else {
        0.52
    };
    painter.rect(
        plot,
        editor_theme::font::CAPTION_SIZE * 0.18,
        plugcat::theme::mix(palette.well, accent, if active { 0.10 } else { 0.055 }),
        egui::Stroke::new(1.0_f32, accent.gamma_multiply(emphasis)),
        egui::StrokeKind::Inside,
    );
    let guide = egui::Stroke::new(
        1.0_f32,
        accent.gamma_multiply(if hovered || active { 0.38 } else { 0.24 }),
    );
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
    let point_radius = editor_theme::font::CAPTION_SIZE
        * if active {
            0.60
        } else if hovered {
            0.54
        } else {
            0.46
        };
    painter.circle_filled(point, point_radius, accent);
    if hovered || active {
        painter.circle_stroke(
            point,
            point_radius * 1.55,
            egui::Stroke::new(1.0_f32, accent.gamma_multiply(emphasis)),
        );
    }
    let label_inset = editor_theme::font::CAPTION_SIZE * 0.65;
    let compact = plot.width() < editor_theme::font::CAPTION_SIZE * 9.5
        || plot.height() < editor_theme::font::CAPTION_SIZE * 6.0;
    let show_labels = plot.width().min(plot.height()) >= editor_theme::font::CAPTION_SIZE * 4.0;
    for (position, align, compact_label, label) in [
        (
            plot.left_top() + egui::Vec2::splat(label_inset),
            egui::Align2::LEFT_TOP,
            "A",
            "ALTR",
        ),
        (
            plot.right_top() + egui::vec2(-label_inset, label_inset),
            egui::Align2::RIGHT_TOP,
            "P",
            "PAIR",
        ),
        (
            plot.left_bottom() + egui::vec2(label_inset, -label_inset),
            egui::Align2::LEFT_BOTTOM,
            "R",
            "RAND",
        ),
        (
            plot.right_bottom() - egui::Vec2::splat(label_inset),
            egui::Align2::RIGHT_BOTTOM,
            "S",
            "SHAPE",
        ),
    ]
    .into_iter()
    .filter(|_| show_labels)
    {
        painter.text(
            position,
            align,
            if compact { compact_label } else { label },
            editor_theme::font::caption(),
            accent.gamma_multiply(if hovered || active { 0.90 } else { 0.64 }),
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
) -> egui::Response {
    let plot = stereo_square_plot(rect);
    let response = ui.interact(plot, id, egui::Sense::click_and_drag());
    response
        .clone()
        .on_hover_text("X selects stereo pattern; Y blends alternate/pair with random/shape");
    let active = response.dragged() || response.is_pointer_button_down_on();
    let point = StereoSquare::new(plot).point(*x, *y);
    let point_hovered = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|pointer| pointer.distance(point) <= editor_theme::title_height(ui) * 0.55);
    if response.hovered() {
        ui.output_mut(|output| {
            output.cursor_icon = if active {
                egui::CursorIcon::Grabbing
            } else if point_hovered {
                egui::CursorIcon::Grab
            } else {
                egui::CursorIcon::Crosshair
            };
        });
    }
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
        (*x, *y) = StereoSquare::new(plot).snap(
            StereoSquare::new(plot).axes_at(constrained),
            snapping,
            editor_theme::title_height(ui) * 0.35,
        );
    }
    *x = (*x).clamp(0.0, 1.0);
    *y = (*y).clamp(0.0, 1.0);
    paint_stereo_square(painter, plot, *x, *y, response.hovered(), active);
    response
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
    plot: egui::Rect,
    center_x: f32,
    data: &PanShapeCurveData,
    hovered: Option<PanShapePointDragTarget>,
    drag: Option<PanShapePointDrag>,
    reveal_handles: bool,
    handle_radius: f32,
) {
    let color = editor_theme::semantic().pan_shape;
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
    let fill_alpha = if reveal_handles { 88 } else { 56 };
    editor_widgets::gradient_area_to_bottom(
        painter,
        &left_points,
        plot.bottom(),
        color,
        fill_alpha,
    );
    editor_widgets::gradient_area_to_bottom(
        painter,
        &right_points,
        plot.bottom(),
        color,
        fill_alpha,
    );
    painter.add(egui::Shape::line(
        left_points,
        egui::Stroke::new(editor_theme::font::CAPTION_SIZE * 0.18, color),
    ));
    painter.add(egui::Shape::line(
        right_points,
        egui::Stroke::new(editor_theme::font::CAPTION_SIZE * 0.18, color),
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
        let center_hovered = hovered == Some(PanShapePointDragTarget::Center);
        let endpoint_active = drag.as_ref().is_some_and(|drag| {
            matches!(drag.target, PanShapePointDragTarget::Endpoint { left: side } if side == left)
        });
        let endpoint_hovered = hovered == Some(PanShapePointDragTarget::Endpoint { left });
        let center = pan_shape_knot_pos(plot, center_x, left, first);
        let endpoint = pan_shape_knot_pos(plot, center_x, left, last);
        if left {
            draw_shape_handle(
                painter,
                center,
                color,
                center_hovered,
                center_active,
                false,
                handle_radius,
            );
        }
        draw_shape_handle(
            painter,
            endpoint,
            color,
            endpoint_hovered,
            endpoint_active,
            false,
            handle_radius,
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
            let knot_hovered = hovered == Some(PanShapePointDragTarget::Knot { left, index });
            draw_shape_handle(
                painter,
                position,
                color,
                knot_hovered,
                knot_active,
                false,
                handle_radius,
            );
        }

        for index in 0..half.knots.len().saturating_sub(1) {
            let curve = pan_shape_curve_handle_pos(plot, center_x, left, half, index);
            let curve_active = drag.as_ref().is_some_and(|drag| {
                matches!(drag.target, PanShapePointDragTarget::Curve { left: side, index: target } if side == left && target == index)
            });
            let curve_hovered = hovered == Some(PanShapePointDragTarget::Curve { left, index });
            if reveal_handles || curve_hovered || curve_active {
                draw_shape_handle(
                    painter,
                    curve,
                    color,
                    curve_hovered,
                    curve_active,
                    true,
                    handle_radius,
                );
            }
        }
    }
}

fn draw_shape_handle(
    painter: &egui::Painter,
    position: egui::Pos2,
    color: egui::Color32,
    hovered: bool,
    active: bool,
    curve: bool,
    base_radius: f32,
) {
    let radius = base_radius
        * if active {
            1.36
        } else if hovered {
            1.18
        } else if curve {
            0.72
        } else {
            0.92
        };
    painter.circle_filled(
        position,
        radius,
        if curve {
            editor_theme::semantic().surface
        } else {
            color.gamma_multiply(if active || hovered { 1.0 } else { 0.76 })
        },
    );
    painter.circle_stroke(
        position,
        radius,
        egui::Stroke::new(
            if active { 1.5_f32 } else { 1.0_f32 },
            color.gamma_multiply(if active || hovered { 1.0 } else { 0.62 }),
        ),
    );
    if active {
        painter.circle_stroke(
            position,
            radius * 1.65,
            egui::Stroke::new(1.0_f32, color.gamma_multiply(0.42)),
        );
    }
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
    let start = pan_shape_knot_pos(plot, center_x, left, start);
    let end = pan_shape_knot_pos(plot, center_x, left, end);
    egui::pos2(
        egui::lerp(start.x..=end.x, segments.seg_cx1[index].clamp(0.0, 1.0)),
        egui::lerp(start.y..=end.y, y),
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

fn pan_shape_hit_target(
    data: &PanShapeCurveData,
    plot: egui::Rect,
    center_x: f32,
    pointer: egui::Pos2,
    radius: f32,
) -> Option<PanShapePointDragTarget> {
    let mut nearest = None;
    let mut consider = |target, position: egui::Pos2| {
        let distance = pointer.distance_sq(position);
        if distance <= radius * radius && nearest.as_ref().is_none_or(|(best, _)| distance < *best)
        {
            nearest = Some((distance, target));
        }
    };

    if let Some(center) = data.left.knots.first().copied() {
        consider(
            PanShapePointDragTarget::Center,
            pan_shape_knot_pos(plot, center_x, true, center),
        );
    }
    for left in [true, false] {
        consider(
            PanShapePointDragTarget::Endpoint { left },
            pan_shape_endpoint(data, plot, center_x, left),
        );
        let half = data.half(left);
        for (index, knot) in half
            .knots
            .iter()
            .copied()
            .enumerate()
            .skip(1)
            .take(half.knots.len().saturating_sub(2))
        {
            consider(
                PanShapePointDragTarget::Knot { left, index },
                pan_shape_knot_pos(plot, center_x, left, knot),
            );
        }
        for index in 0..half.knots.len().saturating_sub(1) {
            consider(
                PanShapePointDragTarget::Curve { left, index },
                pan_shape_curve_handle_pos(plot, center_x, left, half, index),
            );
        }
    }
    nearest.map(|(_, target)| target)
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
