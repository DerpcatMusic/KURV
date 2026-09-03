//! Parameter-bound controls shared by the KURV editor panels.

use truce::params::Params;
use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_modulation::{self, TrackAxis};
use crate::{KurvParams, P, editor_theme};

mod parameter_gesture;

#[cfg(test)]
pub(crate) use parameter_gesture::magnetic_shape_snap;
pub(crate) use parameter_gesture::pointer_gesture_aborted;
pub(crate) use parameter_gesture::{
    ValueSemantic, accumulate_drag, semantic_snap, update_custom_pitch_ratio_drag,
    update_custom_value_drag, update_parameter_drag,
};

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
    let (allocation_id, rect) = ui.allocate_space(egui::vec2(width.max(1.0), height.max(1.0)));
    let interaction = metric_text_bounds(ui, rect, label, value_text);
    let response = ui.interact(
        interaction,
        allocation_id.with("metric-value"),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    crate::editor_shell::register_parameter_hover(ui, id.into(), response.hovered());
    let modulation_gesture = editor_modulation::owns_gesture(ui, state, id, &response);
    let normalized = if modulation_gesture {
        state.get_param(id)
    } else {
        update_parameter_drag(ui, state, id, label, &response)
    };
    editor_modulation::destination(
        ui,
        state,
        id,
        &response,
        normalized,
        rect,
        TrackAxis::Vertical,
    );
    paint_metric_readout_response(ui, rect, label, value_text, accent, &response);
    response.on_hover_text(format!(
        "{label}: drag vertically. Hold Shift for fine control or Ctrl for semantic snap; double-click to reset. Ctrl/Cmd+Shift+Z undoes this parameter; Ctrl/Cmd+Alt+Z redoes it."
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
    let (allocation_id, rect) = ui.allocate_space(egui::vec2(width.max(1.0), height.max(1.0)));
    #[allow(
        clippy::cast_precision_loss,
        reason = "compact source menus have only a handful of values"
    )]
    let last = values.len().saturating_sub(1) as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut current = (state.get_param(id).clamp(0.0, 1.0) * last).round() as usize;
    let interaction = metric_text_bounds(ui, rect, label, values[current.min(values.len() - 1)]);
    let response = ui.interact(
        interaction,
        allocation_id.with("metric-value"),
        egui::Sense::click(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::editor_shell::register_parameter_hover(ui, id.into(), response.hovered());
    if response.double_clicked() {
        if let Some(info) = state
            .params()
            .param_infos()
            .into_iter()
            .find(|info| info.id == u32::from(id))
        {
            let default = info.range.normalize(info.default_plain);
            crate::editor::automate(state, id, default);
            current = (default as f32 * last).round() as usize;
        }
    } else if response.clicked() {
        let next = (current + 1) % values.len();
        #[allow(
            clippy::cast_precision_loss,
            reason = "compact source menus have only a handful of values"
        )]
        crate::editor::automate(
            state,
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
    response.on_hover_text(format!(
        "{label}: click to cycle. Double-click to reset. Ctrl/Cmd+Shift+Z undoes this parameter; Ctrl/Cmd+Alt+Z redoes it."
    ))
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
    let modulated = editor_modulation::response_is_modulated(ui, response);
    paint_metric_readout_visuals(
        ui,
        rect,
        label,
        value,
        accent,
        control_visuals(response, accent),
        response.hovered() || active || response.has_focus() || modulated,
        active,
        modulated,
    );
}

pub(crate) struct MetricTextLayout {
    pub(crate) label: std::sync::Arc<egui::Galley>,
    pub(crate) value: std::sync::Arc<egui::Galley>,
    pub(crate) label_position: egui::Pos2,
    pub(crate) value_position: egui::Pos2,
    pub(crate) value_font: egui::FontId,
}

pub(crate) fn layout_metric_text(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    value: &str,
) -> MetricTextLayout {
    let content_rect = metric_content_rect(rect, 0.0);
    let (label, value, value_font, gap) =
        fitted_metric_galleys(ui, painter, content_rect, label, value);
    let content_height = label.size().y + gap + value.size().y;
    let content_top = content_rect.center().y - content_height * 0.5;
    MetricTextLayout {
        label_position: egui::pos2(content_rect.center().x - label.size().x * 0.5, content_top),
        value_position: egui::pos2(
            content_rect.center().x - value.size().x * 0.5,
            content_top + label.size().y + gap,
        ),
        label,
        value,
        value_font,
    }
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
    modulated: bool,
) {
    let painter = ui.painter_at(rect);
    let layout = layout_metric_text(ui, &painter, rect, label, value);
    let palette = editor_theme::semantic();
    let label_color = if active {
        accent
    } else if modulated {
        accent.gamma_multiply(1.12)
    } else if show_surface {
        accent.gamma_multiply(0.88)
    } else {
        palette.text_muted
    };
    let value_color = if active {
        visuals.value
    } else if modulated {
        accent.gamma_multiply(1.12)
    } else if show_surface {
        accent
    } else {
        palette.text
    };
    painter.galley(layout.label_position, layout.label, label_color);
    painter.galley(layout.value_position, layout.value, value_color);
}

pub(crate) fn metric_text_bounds(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    value: &str,
) -> egui::Rect {
    let painter = ui.painter_at(rect);
    let layout = layout_metric_text(ui, &painter, rect, label, value);
    let label_rect = egui::Rect::from_min_size(layout.label_position, layout.label.size());
    let value_rect = egui::Rect::from_min_size(layout.value_position, layout.value.size());
    label_rect
        .union(value_rect)
        .expand(editor_theme::space::XXS)
        .intersect(rect)
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
    egui::FontId,
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
    let mut label_galley = painter.layout_no_wrap(
        label.to_owned(),
        label_font.clone(),
        egui::Color32::PLACEHOLDER,
    );
    let mut value_galley = painter.layout_no_wrap(
        value.to_owned(),
        value_font.clone(),
        egui::Color32::PLACEHOLDER,
    );
    let content_height = label_galley.size().y + gap + value_galley.size().y;
    let available_height = rect.height().max(1.0);
    if content_height > available_height {
        let scale = available_height / content_height;
        label_font.size *= scale;
        value_font.size *= scale;
        gap *= scale;
        label_galley =
            painter.layout_no_wrap(label.to_owned(), label_font, egui::Color32::PLACEHOLDER);
        value_galley = painter.layout_no_wrap(
            value.to_owned(),
            value_font.clone(),
            egui::Color32::PLACEHOLDER,
        );
    }
    (label_galley, value_galley, value_font, gap)
}
