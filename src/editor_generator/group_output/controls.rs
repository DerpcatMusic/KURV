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
    let open_id = ui.id().with(("group-dropdown-open", id_salt));
    let response = ui
        .interact(
            field_rect,
            ui.id().with(("group-dropdown", id_salt)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut open = ui
        .data(|data| data.get_temp::<bool>(open_id))
        .unwrap_or(false);
    if response.clicked() {
        open = !open;
        ui.data_mut(|data| data.insert_temp(open_id, open));
    }
    let active = response.is_pointer_button_down_on() || open;
    ui.painter().rect_filled(
        field_rect,
        editor_theme::shape::CONTROL_RADIUS,
        if active || response.hovered() {
            palette.control_hover
        } else {
            palette.well
        },
    );
    let arrow_width = field_rect.height() * 0.58;
    let value_rect = egui::Rect::from_min_max(
        field_rect.min,
        egui::pos2(
            (field_rect.right() - arrow_width).max(field_rect.left()),
            field_rect.bottom(),
        ),
    );
    ui.painter().text(
        value_rect.center(),
        egui::Align2::CENTER_CENTER,
        &selected,
        fit_font_to_width(
            ui.painter(),
            &selected,
            editor_theme::font::value(),
            value_rect.width() * 0.90,
        ),
        if active || response.hovered() {
            accent
        } else {
            palette.text
        },
    );
    let arrow_center = egui::pos2(
        field_rect.right() - arrow_width * 0.50,
        field_rect.center().y,
    );
    let arrow_side = arrow_width * 0.24;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            arrow_center + egui::vec2(-arrow_side, -arrow_side * 0.55),
            arrow_center + egui::vec2(arrow_side, -arrow_side * 0.55),
            arrow_center + egui::vec2(0.0, arrow_side * 0.72),
        ],
        if active || response.hovered() {
            accent
        } else {
            palette.text_muted
        },
        egui::Stroke::NONE,
    ));
    if open {
        add_options(ui);
    }
    response
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

pub(super) fn group_envelope_preview(
    ui: &egui::Ui,
    rect: egui::Rect,
    output: crate::generators::GroupOutput,
    accent: egui::Color32,
) {
    let plot = rect.shrink2(egui::vec2(
        editor_theme::space::XXS,
        editor_theme::space::XXS,
    ));
    if plot.width() <= 0.0 || plot.height() <= 0.0 {
        return;
    }
    let time_share = |seconds: f32| 0.10 + 0.20 * (seconds / 20.0).sqrt();
    let attack = time_share(output.attack);
    let decay = time_share(output.decay);
    let release = time_share(output.release);
    let total = (attack + decay + release).max(1.0);
    let attack = attack / total;
    let decay = decay / total;
    let release = release / total;
    let sustain_end = 1.0 - release;
    let bottom = plot.bottom();
    let top = plot.top();
    let sustain = egui::lerp(bottom..=top, output.sustain);
    let point = |x: f32, y: f32| egui::pos2(egui::lerp(plot.left()..=plot.right(), x), y);
    let shape = crate::dsp::curve_progress;
    let mut points = Vec::with_capacity(14);
    points.push(point(0.0, bottom));
    for step in 1..=4 {
        let progress = step as f32 / 4.0;
        points.push(point(
            attack * progress,
            egui::lerp(bottom..=top, shape(progress, output.attack_curve)),
        ));
    }
    for step in 1..=4 {
        let progress = step as f32 / 4.0;
        points.push(point(
            attack + decay * progress,
            egui::lerp(top..=sustain, shape(progress, output.decay_curve)),
        ));
    }
    points.push(point(sustain_end, sustain));
    for step in 1..=4 {
        let progress = step as f32 / 4.0;
        points.push(point(
            sustain_end + release * progress,
            egui::lerp(sustain..=bottom, shape(progress, output.release_curve)),
        ));
    }
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(editor_theme::shape::STROKE, accent.gamma_multiply(0.88)),
    ));
}

fn group_envelope_curve(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl std::hash::Hash,
    curve: &mut f32,
    direction: GroupEnvelopeCurveDirection,
    accent: egui::Color32,
) -> egui::Response {
    let palette = editor_theme::semantic();
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
        // Dragging toward the visible bend should increase the curve value.
        // Falling stages are mirrored vertically, so their gesture polarity
        // must be mirrored as well instead of reusing the attack direction.
        let direction_sign = match direction {
            GroupEnvelopeCurveDirection::Rise => -1.0,
            GroupEnvelopeCurveDirection::Fall => 1.0,
        };
        let delta = response.drag_motion().y * direction_sign;
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
            let shaped = crate::dsp::curve_progress(progress, *curve);
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
    let active = response.is_pointer_button_down_on() || response.dragged();
    let color = if active || response.has_focus() {
        accent
    } else if response.hovered() {
        palette.text
    } else {
        palette.text_muted
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
    let mut value_text = format_value(*value);
    let (rect, response, changed) =
        config_scalar_drag(ui, value, range, speed, default, label, &value_text, size);
    if changed {
        value_text = format_value(*value);
    }
    let active = response.is_pointer_button_down_on() || response.dragged();
    paint_metric_readout_response(
        ui,
        rect,
        label,
        &value_text,
        if active || response.hovered() || response.has_focus() {
            accent
        } else {
            editor_theme::semantic().text
        },
        &response,
    );
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
    if value <= 0.000_001 {
        "−∞ dB".to_owned()
    } else {
        let db = 20.0 * value.log10();
        if db.abs() < 0.05 {
            "0.0 dB".to_owned()
        } else {
            format!("{db:+.1} dB")
        }
    }
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
