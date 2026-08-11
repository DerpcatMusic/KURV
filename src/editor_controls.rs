//! Parameter-bound controls shared by the KURV editor panels.

use truce::params::{FloatParamReadF32, Params};
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_modulation::{self, TrackAxis};
use crate::modulators::routing::ResolvedRouteSource;
use crate::{KurvParams, P, editor_theme};

#[derive(Clone, Copy)]
struct KnobDrag {
    #[cfg(debug_assertions)]
    start: f32,
    value: f32,
    delta_y: f32,
    frames: u32,
}

fn control_visuals(
    response: &egui::Response,
    accent: egui::Color32,
) -> editor_theme::ControlVisuals {
    let active = response.is_pointer_button_down_on() || response.dragged();
    editor_theme::control_visuals(
        response.enabled(),
        response.hovered(),
        active,
        response.has_focus(),
        accent,
    )
}

fn paint_control_frame(
    painter: &egui::Painter,
    rect: egui::Rect,
    visuals: editor_theme::ControlVisuals,
) {
    painter.rect(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        visuals.fill,
        visuals.stroke,
        egui::StrokeKind::Inside,
    );
}

pub(crate) fn pitch_wheel_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let (track, label_rect) = wheel_layout(rect);

    if response.drag_started() {
        state.begin_edit(P::PitchBend);
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let motion = response.drag_motion().y * if fine { 0.2 } else { 1.0 };
        let value = accumulate_drag(state.get_param(P::PitchBend), motion);
        state.set_param(P::PitchBend, f64::from(value));
    }
    if response.drag_stopped() {
        state.set_param(P::PitchBend, 0.5);
        state.end_edit(P::PitchBend);
    } else if response.double_clicked() {
        state.begin_edit(P::PitchBend);
        state.set_param(P::PitchBend, 0.5);
        state.end_edit(P::PitchBend);
    }

    let value = state.get_param(P::PitchBend).clamp(0.0, 1.0);
    let handle_y = egui::lerp(track.bottom()..=track.top(), value);
    let center_y = track.center().y;
    let painter = ui.painter_at(rect);
    let visuals = control_visuals(&response, editor_theme::semantic().primary);
    let track_radius = track.width() * 0.5;
    painter.rect_filled(track, track_radius, visuals.fill);
    painter.rect_stroke(
        track,
        track_radius,
        visuals.stroke,
        egui::StrokeKind::Inside,
    );
    let track_inset = track.width() * 0.14;
    painter.rect_filled(
        egui::Rect::from_x_y_ranges(
            (track.left() + track_inset)..=(track.right() - track_inset),
            handle_y.min(center_y)..=handle_y.max(center_y),
        ),
        track_inset,
        visuals.indicator.gamma_multiply(0.72),
    );
    painter.line_segment(
        [
            egui::pos2(track.left() + track_inset, center_y),
            egui::pos2(track.right() - track_inset, center_y),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, visuals.label),
    );
    painter.circle_filled(
        egui::pos2(track.center().x, handle_y),
        track.width() * 0.29,
        visuals.indicator,
    );
    if let Some(label_rect) = label_rect {
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            "PITCH",
            editor_theme::font::caption(),
            visuals.label,
        );
    }
    response.on_hover_text(
        "Pitch bend: drag vertically. Hold Shift for fine control; releases to center.",
    )
}

/// A latched MIDI modulation wheel. Unlike pitch bend it stays at the last
/// value after release, matching the hardware controller's behavior.
pub(crate) fn mod_wheel_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    let (rect, allocation) = ui.allocate_exact_size(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::hover(),
    );
    let (track, label_rect) = wheel_layout(rect);
    let value_rect = label_rect.map_or(rect, |label| {
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), label.top()))
    });
    let response = ui.interact(
        value_rect,
        allocation.id.with("value"),
        egui::Sense::click_and_drag(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(
            "Mod wheel: drag vertically. Hold Shift for fine control; double-click to reset.",
        );
    if response.drag_started() {
        state.begin_edit(P::ModWheel);
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let motion = response.drag_motion().y * if fine { 0.2 } else { 1.0 };
        let value = accumulate_drag(state.get_param(P::ModWheel), motion);
        state.set_param(P::ModWheel, f64::from(value));
    }
    if response.drag_stopped() {
        state.end_edit(P::ModWheel);
    } else if response.double_clicked() {
        state.begin_edit(P::ModWheel);
        state.set_param(P::ModWheel, 0.0);
        state.end_edit(P::ModWheel);
    }

    let value = state.get_param(P::ModWheel).clamp(0.0, 1.0);
    let handle_y = egui::lerp(track.bottom()..=track.top(), value);
    let painter = ui.painter_at(rect);
    let visuals = control_visuals(&response, editor_theme::semantic().primary);
    let track_radius = track.width() * 0.5;
    painter.rect_filled(track, track_radius, visuals.fill);
    painter.rect_stroke(
        track,
        track_radius,
        visuals.stroke,
        egui::StrokeKind::Inside,
    );
    let track_inset = track.width() * 0.14;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(track.left() + track_inset, handle_y),
            egui::pos2(track.right() - track_inset, track.bottom()),
        ),
        track_inset,
        visuals.indicator.gamma_multiply(0.72),
    );
    painter.circle_filled(
        egui::pos2(track.center().x, handle_y),
        track.width() * 0.29,
        visuals.indicator,
    );
    if let Some(label_rect) = label_rect {
        let jack_size = (label_rect.height() * 0.58).max(editor_theme::shape::FOCUS_STROKE * 2.0);
        let jack_rect = egui::Rect::from_center_size(
            egui::pos2(label_rect.left() + jack_size * 0.5, label_rect.center().y),
            egui::vec2(jack_size, jack_size),
        );
        let mut jack_response = ui.interact(
            label_rect,
            allocation.id.with("source"),
            egui::Sense::drag(),
        );
        jack_response.rect = jack_rect;
        let _ = editor_modulation::source_handle_for(
            ui,
            state,
            ResolvedRouteSource::ModWheel,
            "MOD WHEEL",
            &jack_response,
        );
        let text_rect = egui::Rect::from_min_max(
            egui::pos2(
                jack_rect.right() + editor_theme::space::XXS,
                label_rect.top(),
            ),
            label_rect.right_bottom(),
        );
        painter.text(
            text_rect.center(),
            egui::Align2::CENTER_CENTER,
            "MOD",
            editor_theme::font::caption(),
            visuals.label,
        );
    }
    response
}

fn wheel_layout(rect: egui::Rect) -> (egui::Rect, Option<egui::Rect>) {
    let padding = editor_theme::space::XXS;
    let label_height = editor_theme::font::CAPTION_SIZE + padding;
    let show_label =
        rect.height() >= editor_theme::space::LG + label_height + editor_theme::space::XS;
    let label_rect = show_label.then(|| {
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - label_height),
            rect.right_bottom(),
        )
    });
    let inset_x = padding.min(rect.width() * 0.25);
    let inset_y = padding.min(rect.height() * 0.25);
    let track_top = rect.top() + inset_y;
    let track_bottom = label_rect
        .map_or(rect.bottom() - inset_y, |label| label.top() - padding)
        .max(track_top + editor_theme::shape::STROKE);
    let track_area = egui::Rect::from_min_max(
        egui::pos2(rect.left() + inset_x, track_top),
        egui::pos2(rect.right() - inset_x, track_bottom),
    );
    let width = (editor_theme::space::SM + editor_theme::space::XS)
        .min(track_area.width())
        .max(editor_theme::shape::STROKE);
    let track = egui::Rect::from_center_size(
        track_area.center(),
        egui::vec2(width, track_area.height().max(editor_theme::shape::STROKE)),
    );
    (track, label_rect)
}

pub(crate) fn param_field_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    width: f32,
    height: f32,
) -> egui::Response {
    param_field_sized_value(ui, state, id, label, width, height, None)
}

pub(crate) fn fit_font_to_width(
    painter: &egui::Painter,
    text: &str,
    mut font: egui::FontId,
    width: f32,
) -> egui::FontId {
    let measured = painter
        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
        .size()
        .x;
    if measured > width.max(1.0) {
        font.size *= width.max(1.0) / measured;
    }
    font
}

pub(crate) fn param_field_sized_value(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    width: f32,
    height: f32,
    value_text: Option<&str>,
) -> egui::Response {
    let size = egui::vec2(width.max(1.0), height.max(1.0));
    let portrait = size.y > size.x * 1.15;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let modulation_gesture = editor_modulation::owns_gesture(ui, state, id, &response);
    let value = if modulation_gesture {
        state.get_param(id)
    } else {
        update_parameter_drag(ui, state, id, label, &response)
    };
    let painter = ui.painter_at(rect);
    let visuals = control_visuals(&response, editor_theme::semantic().primary);
    if response.hovered()
        || response.dragged()
        || response.has_focus()
        || response.is_pointer_button_down_on()
        || modulation_gesture
    {
        paint_control_frame(&painter, rect, visuals);
    }
    let interior = rect.shrink(editor_theme::shape::STROKE);
    let mut portrait_fill = None;
    if portrait {
        let value_y = egui::lerp(interior.bottom()..=interior.top(), value);
        let center = bipolar_center(state, id);
        let anchor_y = center.map_or(interior.bottom(), |center| {
            egui::lerp(interior.bottom()..=interior.top(), center)
        });
        let fill = egui::Rect::from_x_y_ranges(
            interior.x_range(),
            value_y.min(anchor_y)..=value_y.max(anchor_y),
        );
        painter.rect_filled(fill, 0.0, visuals.indicator);
        portrait_fill = Some(fill);
        if center.is_some() {
            painter.line_segment(
                [
                    egui::pos2(interior.left(), anchor_y),
                    egui::pos2(interior.right(), anchor_y),
                ],
                egui::Stroke::new(
                    editor_theme::shape::STROKE,
                    visuals.indicator.gamma_multiply(0.55),
                ),
            );
        }
    } else {
        let progress_height = editor_theme::shape::STROKE * 2.0;
        let progress = egui::Rect::from_min_max(
            egui::pos2(interior.left(), interior.bottom() - progress_height),
            egui::pos2(
                egui::lerp(interior.left()..=interior.right(), value),
                interior.bottom(),
            ),
        );
        painter.rect_filled(progress, editor_theme::shape::STROKE, visuals.indicator);
    }
    let value_text = value_text
        .map(str::to_owned)
        .unwrap_or_else(|| compact_param_value(state, id));
    let progress_height = if portrait {
        0.0
    } else {
        editor_theme::shape::STROKE * 2.0
    };
    let content_rect = metric_content_rect(rect, progress_height);
    let split_height = editor_theme::font::CAPTION_SIZE
        + editor_theme::font::VALUE_SIZE
        + editor_theme::compact_gap(ui)
        + editor_theme::shape::STROKE;
    if content_rect.height() >= split_height {
        let (label_galley, value_galley, gap) =
            fitted_metric_galleys(ui, &painter, content_rect, label, &value_text);
        let content_height = label_galley.size().y + gap + value_galley.size().y;
        let content_top = content_rect.center().y - content_height * 0.5;
        let label_position = egui::pos2(
            content_rect.center().x - label_galley.size().x * 0.5,
            content_top,
        );
        let value_position = egui::pos2(
            content_rect.center().x - value_galley.size().x * 0.5,
            content_top + label_galley.size().y + gap,
        );
        let label_sample = label_position + label_galley.size() * 0.5;
        let value_sample = value_position + value_galley.size() * 0.5;
        let text_on_fill = |position| {
            portrait_fill
                .filter(|fill| fill.contains(position))
                .map_or(visuals.label, |_| {
                    editor_theme::readable_text(visuals.indicator)
                })
        };
        painter.galley(label_position, label_galley, text_on_fill(label_sample));
        painter.galley(
            value_position,
            value_galley,
            portrait_fill
                .filter(|fill| fill.contains(value_sample))
                .map_or_else(
                    || visuals.value,
                    |_| editor_theme::readable_text(visuals.indicator),
                ),
        );
    } else {
        let combined = format!("{label} {value_text}");
        let available_width = content_rect.width().max(1.0);
        let combined_width = painter
            .layout_no_wrap(
                combined.clone(),
                editor_theme::font::value(),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let text = if combined_width <= available_width {
            combined
        } else {
            value_text
        };
        let mut font = fit_font_to_width(
            &painter,
            &text,
            editor_theme::font::value(),
            available_width,
        );
        font.size = font.size.min(content_rect.height().max(1.0));
        painter.text(
            content_rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font,
            visuals.value,
        );
    }
    editor_modulation::destination(
        ui,
        state,
        id,
        &response,
        value,
        rect,
        if portrait {
            TrackAxis::Vertical
        } else {
            TrackAxis::Horizontal
        },
    );
    response.on_hover_text(format!(
        "{label}: drag vertically. Hold Shift for fine control; double-click to reset."
    ))
}

pub(crate) fn metric_param_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    value_text: &str,
    width: f32,
    height: f32,
    accent: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    let modulation_gesture = editor_modulation::owns_gesture(ui, state, id, &response);
    let normalized = if modulation_gesture {
        state.get_param(id)
    } else {
        update_parameter_drag(ui, state, id, label, &response)
    };
    paint_metric_readout_response(ui, rect, label, value_text, accent, &response);
    editor_modulation::destination(
        ui,
        state,
        id,
        &response,
        normalized,
        rect,
        TrackAxis::Vertical,
    );
    response.on_hover_text(format!(
        "{label}: drag vertically. Hold Shift for fine control; double-click to reset."
    ))
}

pub(crate) fn metric_enum_readout(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    values: &[&str],
    width: f32,
    height: f32,
    accent: egui::Color32,
) -> egui::Response {
    debug_assert!(!values.is_empty());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(1.0), height.max(1.0)),
        egui::Sense::click(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    #[allow(
        clippy::cast_precision_loss,
        reason = "compact source menus have only a handful of values"
    )]
    let last = values.len().saturating_sub(1) as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut current = (state.get_param(id).clamp(0.0, 1.0) * last).round() as usize;
    if response.double_clicked() {
        if let Some(info) = state
            .params()
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(id))
        {
            let default = info.range.normalize(info.default_plain);
            state.automate(id, default);
            current = (default as f32 * last).round() as usize;
        }
    } else if response.clicked() {
        let next = (current + 1) % values.len();
        #[allow(
            clippy::cast_precision_loss,
            reason = "compact source menus have only a handful of values"
        )]
        state.automate(
            id,
            if last > 0.0 {
                next as f64 / f64::from(last)
            } else {
                0.0
            },
        );
        current = next;
    }
    paint_metric_readout_response(
        ui,
        rect,
        label,
        values[current.min(values.len() - 1)],
        accent,
        &response,
    );
    response.on_hover_text(format!("{label}: click to cycle. Double-click to reset."))
}

pub(crate) fn paint_metric_readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    accent: egui::Color32,
    active: bool,
) {
    let hovered = ui.rect_contains_pointer(rect);
    let visuals = editor_theme::control_visuals(ui.is_enabled(), hovered, active, false, accent);
    paint_metric_readout_visuals(
        ui,
        rect,
        label,
        value,
        accent,
        visuals,
        hovered || active,
        active,
    );
}

pub(crate) fn paint_metric_readout_response(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    accent: egui::Color32,
    response: &egui::Response,
) {
    let active = response.is_pointer_button_down_on() || response.dragged();
    paint_metric_readout_visuals(
        ui,
        rect,
        label,
        value,
        accent,
        control_visuals(response, accent),
        response.hovered() || active || response.has_focus(),
        active,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_metric_readout_visuals(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
    accent: egui::Color32,
    visuals: editor_theme::ControlVisuals,
    show_surface: bool,
    active: bool,
) {
    let painter = ui.painter_at(rect);
    if show_surface {
        paint_control_frame(&painter, rect, visuals);
    }
    let content_rect = metric_content_rect(rect, 0.0);
    let (label_galley, value_galley, gap) =
        fitted_metric_galleys(ui, &painter, content_rect, label, value);
    let content_height = label_galley.size().y + gap + value_galley.size().y;
    let content_top = content_rect.center().y - content_height * 0.5;
    let label_position = egui::pos2(
        content_rect.center().x - label_galley.size().x * 0.5,
        content_top,
    );
    let value_position = egui::pos2(
        content_rect.center().x - value_galley.size().x * 0.5,
        content_top + label_galley.size().y + gap,
    );
    painter.galley(label_position, label_galley, visuals.label);
    painter.galley(
        value_position,
        value_galley,
        if active {
            visuals.value
        } else {
            accent.gamma_multiply(if show_surface { 0.94 } else { 0.82 })
        },
    );
}

fn metric_content_rect(rect: egui::Rect, reserved_bottom: f32) -> egui::Rect {
    let inset_x = editor_theme::space::XXS.min(rect.width() * 0.5);
    let inset_y = editor_theme::shape::STROKE.min(rect.height() * 0.5);
    let mut content = rect.shrink2(egui::vec2(inset_x, inset_y));
    content.max.y = (content.max.y - reserved_bottom).max(content.min.y);
    content
}

fn fitted_metric_galleys(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    value: &str,
) -> (
    std::sync::Arc<egui::Galley>,
    std::sync::Arc<egui::Galley>,
    f32,
) {
    let available_width = rect.width().max(1.0);
    let mut label_font = fit_font_to_width(
        painter,
        label,
        editor_theme::font::caption(),
        available_width,
    );
    let mut value_font =
        fit_font_to_width(painter, value, editor_theme::font::value(), available_width);
    let mut gap = editor_theme::compact_gap(ui);
    let mut label_galley =
        painter.layout_no_wrap(label.to_owned(), label_font.clone(), egui::Color32::WHITE);
    let mut value_galley =
        painter.layout_no_wrap(value.to_owned(), value_font.clone(), egui::Color32::WHITE);
    let content_height = label_galley.size().y + gap + value_galley.size().y;
    let available_height = rect.height().max(1.0);
    if content_height > available_height {
        let scale = available_height / content_height;
        label_font.size *= scale;
        value_font.size *= scale;
        gap *= scale;
        label_galley = painter.layout_no_wrap(label.to_owned(), label_font, egui::Color32::WHITE);
        value_galley = painter.layout_no_wrap(value.to_owned(), value_font, egui::Color32::WHITE);
    }
    (label_galley, value_galley, gap)
}

fn bipolar_center(state: &PluginContext<KurvParams>, id: P) -> Option<f32> {
    state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == u32::from(id))
        .filter(|info| info.range.min() < 0.0 && info.range.max() > 0.0)
        .map(|info| info.range.normalize(0.0) as f32)
}

fn compact_param_value(state: &PluginContext<KurvParams>, id: P) -> String {
    if id == P::Shape {
        let shape = state.params().shape.value();
        let rounded = shape.round();
        if (shape - rounded).abs() < 0.01 {
            match rounded {
                value if value < 0.5 => "SINE",
                value if value < 1.5 => "TRI",
                value if value < 2.5 => "SAW",
                _ => "PULSE",
            }
            .to_owned()
        } else {
            format!("{shape:.2}")
        }
    } else {
        state.format_param(id)
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "Truce normalized parameters are bounded to 0..1 before entering egui's f32 controls"
)]
fn update_parameter_drag(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    id: P,
    label: &str,
    response: &egui::Response,
) -> f32 {
    let raw_id = u32::from(id);
    let origin_id = response.id.with("drag_origin");
    let mut value = state.get_param(id);
    let info = state
        .params()
        .param_infos()
        .into_iter()
        .find(|info| info.id == raw_id);

    if response.double_clicked()
        && let Some(info) = info
    {
        value = info.range.normalize(info.default_plain) as f32;
        state.begin_edit(id);
        state.set_param(id, f64::from(value));
        state.end_edit(id);
        return value;
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    if response.has_focus() && !response.dragged() && !response.is_pointer_button_down_on() {
        let direction = ui.input(|input| {
            i8::from(
                input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::ArrowRight),
            ) - i8::from(
                input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowLeft),
            )
        });
        if direction != 0 {
            let fine = ui.input(|input| input.modifiers.shift);
            let step = info.and_then(|info| info.range.step_count()).map_or(
                if fine { 0.001 } else { 0.01 },
                |steps| {
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "parameter step counts fit exactly in the compact control ranges"
                    )]
                    let count = steps.get() as f32;
                    count.recip()
                },
            );
            let next = (value + f32::from(direction) * step).clamp(0.0, 1.0);
            if (next - value).abs() > f32::EPSILON {
                value = next;
                state.begin_edit(id);
                state.set_param(id, f64::from(value));
                state.end_edit(id);
            }
        }
    }
    if response.drag_started() {
        state.begin_edit(id);
        ui.data_mut(|data| {
            data.insert_temp(
                origin_id,
                KnobDrag {
                    #[cfg(debug_assertions)]
                    start: value,
                    value,
                    delta_y: 0.0,
                    frames: 0,
                },
            );
        });
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let motion = response.drag_motion().y * if fine { 0.2 } else { 1.0 };
        let mut drag = ui
            .data_mut(|data| data.get_temp::<KnobDrag>(origin_id))
            .unwrap_or(KnobDrag {
                #[cfg(debug_assertions)]
                start: value,
                value,
                delta_y: 0.0,
                frames: 0,
            });
        let discrete_semitone_drag = info
            .filter(|_| is_integer_semitone_parameter(id))
            .and_then(|info| info.range.step_count())
            .map_or(0.0, |steps| steps.get() as f32 * 8.0);
        drag.value = if discrete_semitone_drag > 0.0 {
            (drag.value - motion / discrete_semitone_drag).clamp(0.0, 1.0)
        } else {
            accumulate_drag(drag.value, motion)
        };
        drag.delta_y += motion;
        drag.frames += 1;
        ui.data_mut(|data| data.insert_temp(origin_id, drag));
        let unrounded = drag.value;
        let next = if !fine && id == P::Shape {
            magnetic_shape_snap(unrounded)
        } else {
            info.and_then(|info| info.range.step_count())
                .map_or(unrounded, |steps| {
                    #[allow(clippy::cast_precision_loss, reason = "parameter step counts are tiny")]
                    let count = steps.get() as f32;
                    (unrounded * count).round() / count
                })
        };
        if (next - value).abs() > f32::EPSILON {
            value = next;
            state.set_param(id, f64::from(value));
        }
    }
    if response.drag_stopped() {
        state.end_edit(id);
        let drag = ui.data_mut(|data| {
            let drag = data.get_temp::<KnobDrag>(origin_id);
            data.remove::<KnobDrag>(origin_id);
            drag
        });
        log_knob_gesture(label, drag, state.get_param(id));
    }
    value
}

fn is_integer_semitone_parameter(id: P) -> bool {
    matches!(
        id,
        P::Transpose | P::Osc1Transpose | P::Osc2Transpose | P::Osc3Transpose
    )
}

pub(crate) fn accumulate_drag(value: f32, delta_y: f32) -> f32 {
    (value - delta_y / 150.0).clamp(0.0, 1.0)
}

pub(crate) fn magnetic_shape_snap(value: f32) -> f32 {
    [0.0_f32, 1.0 / 3.0, 2.0 / 3.0, 1.0]
        .into_iter()
        .find(|point| (value - point).abs() <= 0.018)
        .unwrap_or(value)
}

#[cfg(debug_assertions)]
#[allow(
    clippy::print_stderr,
    reason = "debug builds log UI gestures to diagnose host pointer behavior"
)]
fn log_knob_gesture(label: &str, drag: Option<KnobDrag>, end: f32) {
    if let Some(drag) = drag {
        eprintln!(
            "[KURV UI] knob: {label} start={:.4} end={end:.4} delta_y={:.2} frames={}",
            drag.start, drag.delta_y, drag.frames
        );
    }
}

#[cfg(not(debug_assertions))]
fn log_knob_gesture(_label: &str, _drag: Option<KnobDrag>, _end: f32) {}
