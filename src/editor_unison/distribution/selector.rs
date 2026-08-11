//! Alignment mode and vertical selector controls.

use crate::editor_theme;
use crate::voices::UnisonAlignmentMode;

pub(super) fn compact_alignment_mode_combo(
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

pub(super) fn paint_vertical_selector_state(
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
