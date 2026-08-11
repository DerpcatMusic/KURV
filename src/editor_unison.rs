//! Unison distribution, stereo blend, and direct point-shaper views.

use truce_core::editor::PluginContext;

use crate::generators::{ModuleId, OscillatorSlot};
use crate::modulators::routing::{ModulationRouteTarget, OscillatorControl};
use crate::pan_curve::PanShapeCurveState;
use crate::voices::PanShapeSettings;
use crate::voices::{
    JITTER_EXCURSION_CENTS, MAX_UNISON, SwarmMode, UnisonAlignmentMode, UnisonSettings,
    fill_oscillator_unison_layout, fill_unison_jitter_offsets_mode, unison_static_pitch_cents,
};
use crate::{KurvParams, editor_theme};

mod pan_panel;
mod pan_shape;

pub(crate) use pan_panel::custom_pan_panel_view;

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
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    normalized: f32,
    changed: bool,
) {
    if let Some((_, param, _)) =
        crate::editor_modulation::host_automation_binding(ui, state, target)
    {
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
    let response = ui.interact(
        unison_plot,
        distribution_id,
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
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
            egui::Sense::CLICK | egui::Sense::DRAG,
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
        ui,
        state,
        amount_target,
        &response,
        amount,
        before.0 != config.unison_amount.to_bits(),
    );
    update_host_axis(
        ui,
        state,
        curve_target,
        &response,
        curve,
        before.1 != config.unison_curve.to_bits(),
    );
    update_host_axis(
        ui,
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
