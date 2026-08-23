//! Dedicated two-axis performance modulation source.

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::pointer_gesture_aborted;
use crate::editor_modulation;
use crate::modulators::routing::ResolvedRouteSource;
use crate::{KurvParams, P, editor_theme};

pub(super) fn xy_source_pad(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    let minimum = editor_theme::shape::STROKE;
    let (rect, allocation) = ui.allocate_exact_size(
        egui::vec2(width.max(minimum), height.max(minimum)),
        egui::Sense::hover(),
    );
    let footer_height =
        (editor_theme::font::CAPTION_SIZE + editor_theme::space::XS).min(rect.height() * 0.28);
    let surface = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(
            rect.right(),
            (rect.bottom() - footer_height).max(rect.top()),
        ),
    )
    .shrink(editor_theme::space::XXS);
    let response = ui
        .interact(
            surface,
            allocation.id.with("xy-value"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Crosshair)
        .on_hover_text(
            "Dedicated XY modulation source. Drag the pad to automate both axes; drag the X or Y jack onto a destination.",
        );
    update_xy_value(ui, state, &response, surface);
    paint_xy_pad(ui, state, surface, &response);
    paint_source_jacks(ui, state, allocation.id, rect, surface.bottom());
    response
}

fn update_xy_value(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    response: &egui::Response,
    surface: egui::Rect,
) {
    let edit_id = response.id.with("host-edit");
    if response.drag_started() {
        begin_xy_edit(state);
        ui.data_mut(|data| data.insert_temp(edit_id, true));
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        set_xy_from_pointer(state, surface, pointer);
    }
    let edit_active = ui.data(|data| data.get_temp::<bool>(edit_id).unwrap_or(false));
    if edit_active && (response.drag_stopped() || pointer_gesture_aborted(ui)) {
        end_xy_edit(state);
        ui.data_mut(|data| data.remove::<bool>(edit_id));
    } else if response.double_clicked() {
        begin_xy_edit(state);
        state.set_param(P::XySourceX, 0.5);
        state.set_param(P::XySourceY, 0.5);
        end_xy_edit(state);
    } else if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        begin_xy_edit(state);
        set_xy_from_pointer(state, surface, pointer);
        end_xy_edit(state);
    }
}

fn begin_xy_edit(state: &PluginContext<KurvParams>) {
    crate::editor::begin_edit(state, P::XySourceX);
    crate::editor::begin_edit(state, P::XySourceY);
}

fn end_xy_edit(state: &PluginContext<KurvParams>) {
    crate::editor::end_edit(state, P::XySourceY);
    crate::editor::end_edit(state, P::XySourceX);
}

fn set_xy_from_pointer(
    state: &PluginContext<KurvParams>,
    surface: egui::Rect,
    pointer: egui::Pos2,
) {
    let x = ((pointer.x - surface.left()) / surface.width().max(f32::EPSILON)).clamp(0.0, 1.0);
    let y = ((surface.bottom() - pointer.y) / surface.height().max(f32::EPSILON)).clamp(0.0, 1.0);
    state.set_param(P::XySourceX, f64::from(x));
    state.set_param(P::XySourceY, f64::from(y));
}

fn paint_xy_pad(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    surface: egui::Rect,
    response: &egui::Response,
) {
    if !surface.is_positive() {
        return;
    }
    let palette = editor_theme::semantic();
    let x = state.get_param(P::XySourceX).clamp(0.0, 1.0);
    let y = state.get_param(P::XySourceY).clamp(0.0, 1.0);
    let x_color = editor_modulation::source_color(crate::modulators::state::MAX_MODULATION_SOURCES);
    let y_color =
        editor_modulation::source_color(crate::modulators::state::MAX_MODULATION_SOURCES + 1);
    let painter = ui.painter_at(surface);
    painter.rect_filled(surface, editor_theme::shape::CONTROL_RADIUS, palette.well);
    painter.rect_stroke(
        surface,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(
            if response.has_focus() {
                editor_theme::shape::FOCUS_STROKE
            } else {
                editor_theme::shape::STROKE
            },
            if response.hovered() {
                palette.text_muted
            } else {
                palette.grid
            },
        ),
        egui::StrokeKind::Inside,
    );
    let center = egui::pos2(
        egui::lerp(surface.left()..=surface.right(), x),
        egui::lerp(surface.bottom()..=surface.top(), y),
    );
    painter.line_segment(
        [
            egui::pos2(center.x, surface.top()),
            egui::pos2(center.x, surface.bottom()),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, x_color.gamma_multiply(0.48)),
    );
    painter.line_segment(
        [
            egui::pos2(surface.left(), center.y),
            egui::pos2(surface.right(), center.y),
        ],
        egui::Stroke::new(editor_theme::shape::STROKE, y_color.gamma_multiply(0.48)),
    );
    painter.circle_filled(center, editor_theme::space::XS, palette.surface);
    painter.circle_stroke(
        center,
        editor_theme::space::XS,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, palette.text),
    );
}

fn paint_source_jacks(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    allocation_id: egui::Id,
    rect: egui::Rect,
    footer_top: f32,
) {
    let footer = egui::Rect::from_min_max(egui::pos2(rect.left(), footer_top), rect.right_bottom());
    let half = footer.width() * 0.5;
    for (offset, source, label) in [
        (0.0, ResolvedRouteSource::XyX, "XY X"),
        (half, ResolvedRouteSource::XyY, "XY Y"),
    ] {
        let jack_rect = egui::Rect::from_min_size(
            egui::pos2(footer.left() + offset, footer.top()),
            egui::vec2(half, footer.height()),
        );
        let response = ui.interact(jack_rect, allocation_id.with(label), egui::Sense::drag());
        let _ = editor_modulation::source_handle_for(ui, state, source, label, &response);
    }
}
