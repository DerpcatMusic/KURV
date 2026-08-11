use crate::editor_controls::{fit_font_to_width, paint_metric_readout_response};
use crate::editor_theme;
use crate::editor_widgets::with_child;

use super::super::{config_scalar_drag, format_pan};

pub(super) fn group_dropdown_readout(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash + Copy,
    label: &str,
    selected: String,
    accent: egui::Color32,
    add_options: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let palette = editor_theme::semantic();
    let gap = editor_theme::space::XXS;
    let label_width = (rect.width() * 0.39 - gap * 0.5).max(0.0);
    let label_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.left() + label_width, rect.bottom()),
    );
    let field_rect = egui::Rect::from_min_max(
        egui::pos2(
            (label_rect.right() + gap).min(rect.right()),
            rect.top() + editor_theme::space::XXS,
        ),
        egui::pos2(rect.right(), rect.bottom() - editor_theme::space::XXS),
    );
    ui.painter().text(
        label_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        fit_font_to_width(
            ui.painter(),
            label,
            editor_theme::font::caption(),
            label_rect.width() * 0.92,
        ),
        palette.text_muted,
    );
    ui.painter().rect_filled(
        field_rect,
        editor_theme::shape::CONTROL_RADIUS,
        palette.control,
    );
    ui.painter().rect_stroke(
        field_rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette.grid.gamma_multiply(0.72),
        ),
        egui::StrokeKind::Inside,
    );
    let mut response = None;
    with_child(
        ui,
        field_rect,
        ("group-dropdown", id_salt),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.spacing_mut().button_padding = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size.y = field_rect.height();
            ui.visuals_mut().override_text_color = Some(accent);
            ui.visuals_mut().widgets.inactive.bg_fill = palette.control;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = palette.control;
            ui.visuals_mut().widgets.hovered.bg_fill = palette.control_hover;
            ui.visuals_mut().widgets.active.bg_fill = palette.control_hover;
            ui.visuals_mut().widgets.hovered.fg_stroke.color = accent;
            ui.visuals_mut().widgets.active.fg_stroke.color = accent;
            response = Some(
                egui::ComboBox::from_id_salt(("group-dropdown-combo", id_salt))
                    .selected_text(selected)
                    .width(field_rect.width())
                    .show_ui(ui, add_options)
                    .response,
            );
        },
    );
    response
        .unwrap_or_else(|| ui.interact(field_rect, ui.id().with(id_salt), egui::Sense::hover()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
pub(super) enum GroupEnvelopeCurveDirection {
    Rise,
    Fall,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn group_envelope_control(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash + Copy,
    value: &mut f32,
    curve: &mut f32,
    label: &str,
    direction: GroupEnvelopeCurveDirection,
    default: f32,
    format_value: fn(f32) -> String,
    accent: egui::Color32,
) -> (egui::Response, egui::Response) {
    let gap = editor_theme::space::XXS;
    let curve_width = (rect.height() * 0.58).min(rect.width() * 0.27);
    let readout_width = (rect.height() * 1.08).min(rect.width() - curve_width - gap);
    let cluster_width = (readout_width + gap + curve_width).min(rect.width());
    let cluster =
        egui::Rect::from_center_size(rect.center(), egui::vec2(cluster_width, rect.height()));
    let readout = egui::Rect::from_min_max(
        cluster.min,
        egui::pos2(
            (cluster.right() - curve_width - gap).max(cluster.left()),
            cluster.bottom(),
        ),
    );
    let curve_rect = egui::Rect::from_min_max(
        egui::pos2(readout.right() + gap, cluster.top()),
        cluster.max,
    );
    let mut value_response = None;
    with_child(
        ui,
        readout,
        ("group-envelope-value", id_salt),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let (_, response) = group_scalar_readout(
                ui,
                value,
                label,
                0.0..=20.0,
                0.01,
                default,
                readout.size(),
                format_value,
                accent,
            );
            value_response = Some(response);
        },
    );
    let curve_response = group_envelope_curve(ui, curve_rect, id_salt, curve, direction, accent);
    (
        value_response
            .unwrap_or_else(|| ui.interact(readout, ui.id().with(id_salt), egui::Sense::hover())),
        curve_response,
    )
}

fn group_envelope_curve(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    curve: &mut f32,
    direction: GroupEnvelopeCurveDirection,
    accent: egui::Color32,
) -> egui::Response {
    let interaction = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2(rect.width(), rect.height() * 0.88),
    );
    let response = ui
        .interact(
            interaction,
            egui::Id::new(("group-envelope-curve", id_salt)),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Drag to bend the envelope stage; double-click to reset.");
    if response.dragged() {
        let delta = -response.drag_motion().y;
        let precision = if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        *curve = (*curve
            + delta * precision / interaction.height().max(editor_theme::shape::STROKE))
        .clamp(-1.0, 1.0);
    } else if response.double_clicked() {
        *curve = 0.0;
    }

    let show_value = response.hovered() || response.dragged();
    let glyph_side = rect.width().min(rect.height() * 0.58);
    let glyph_center = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.43);
    let glyph = egui::Rect::from_center_size(glyph_center, egui::vec2(glyph_side, glyph_side));
    let plot = egui::Rect::from_min_max(
        glyph.left_top() + egui::vec2(glyph.width() * 0.08, glyph.height() * 0.08),
        glyph.right_bottom() - egui::vec2(glyph.width() * 0.08, glyph.height() * 0.08),
    );
    let points = (0..=12)
        .map(|index| {
            let progress = index as f32 / 12.0;
            let shaped = progress + curve.clamp(-1.0, 1.0) * progress * (1.0 - progress);
            let y = match direction {
                GroupEnvelopeCurveDirection::Rise => 1.0 - shaped,
                GroupEnvelopeCurveDirection::Fall => shaped,
            };
            egui::pos2(
                egui::lerp(plot.left()..=plot.right(), progress),
                egui::lerp(plot.top()..=plot.bottom(), y),
            )
        })
        .collect();
    let color = if response.is_pointer_button_down_on() {
        ui.visuals().text_color()
    } else {
        accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.78 })
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(
            (rect.height() * 0.034).max(editor_theme::shape::STROKE),
            color,
        ),
    ));
    if show_value {
        let text = format!("{:+.0}%", *curve * 100.0);
        ui.painter().text(
            egui::pos2(rect.center().x, rect.bottom() - rect.height() * 0.04),
            egui::Align2::CENTER_BOTTOM,
            &text,
            fit_font_to_width(
                ui.painter(),
                &text,
                editor_theme::font::caption(),
                rect.width() * 0.95,
            ),
            color,
        );
    }
    response
}

#[allow(clippy::too_many_arguments)]
pub(super) fn group_scalar_readout(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
    speed: f32,
    default: f32,
    size: egui::Vec2,
    format_value: fn(f32) -> String,
    accent: egui::Color32,
) -> (egui::Rect, egui::Response) {
    let (rect, response, _) = config_scalar_drag(ui, value, range, speed, default, size);
    let value_text = format_value(*value);
    let active = response.is_pointer_button_down_on() || response.dragged();
    paint_metric_readout_response(ui, rect, label, &value_text, accent, &response);
    let track = egui::Rect::from_min_max(
        egui::pos2(
            rect.left(),
            rect.bottom() - editor_theme::shape::FOCUS_STROKE,
        ),
        rect.right_bottom(),
    );
    if response.hovered() || active {
        ui.painter().rect_filled(
            track,
            0.0,
            accent.gamma_multiply(if active { 0.92 } else { 0.48 }),
        );
    }
    (track, response)
}

pub(super) fn format_gain(value: f32) -> String {
    format!("{value:.2}")
}

pub(super) fn format_pan_value(value: f32) -> String {
    format_pan(value)
}

pub(super) fn format_seconds(value: f32) -> String {
    format!("{:.0} ms", value * 1_000.0)
}

pub(super) fn format_percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

pub(super) fn output_pair_label(pair: u8) -> String {
    let left = usize::from(pair) * 2 + 1;
    format!("OUT {left}/{}", left + 1)
}
