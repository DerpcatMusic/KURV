use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{accumulate_drag, pointer_gesture_aborted};
use crate::editor_modulation;
use crate::modulators::routing::ResolvedRouteSource;
use crate::{KurvParams, P, editor_theme};

#[derive(Clone, Copy)]
enum WheelKind {
    Pitch,
    Mod,
}

impl WheelKind {
    const fn param(self) -> P {
        match self {
            Self::Pitch => P::PitchBend,
            Self::Mod => P::ModWheel,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Pitch => "PITCH",
            Self::Mod => "MOD",
        }
    }

    const fn reset_value(self) -> f64 {
        match self {
            Self::Pitch => 0.5,
            Self::Mod => 0.0,
        }
    }

    const fn springs_to_center(self) -> bool {
        matches!(self, Self::Pitch)
    }

    const fn tooltip(self) -> &'static str {
        match self {
            Self::Pitch => {
                "Pitch bend: drag vertically. Hold Shift for fine control; releases to center."
            }
            Self::Mod => {
                "Mod wheel: drag vertically. Hold Shift for fine control; double-click to reset."
            }
        }
    }
}

pub(super) fn pitch_wheel_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    wheel(ui, state, WheelKind::Pitch, width, height)
}

pub(super) fn mod_wheel_sized(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    width: f32,
    height: f32,
) -> egui::Response {
    wheel(ui, state, WheelKind::Mod, width, height)
}

fn wheel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    kind: WheelKind,
    width: f32,
    height: f32,
) -> egui::Response {
    let minimum = editor_theme::shape::STROKE;
    let (rect, allocation) = ui.allocate_exact_size(
        egui::vec2(width.max(minimum), height.max(minimum)),
        egui::Sense::hover(),
    );
    let (surface, label_rect) = wheel_layout(rect);
    let response = ui
        .interact(
            surface,
            allocation.id.with("value"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(kind.tooltip());
    crate::editor_shell::register_parameter_hover(ui, kind.param().into(), response.hovered());
    update_wheel_value(ui, state, kind, &response);

    let value = state.get_param(kind.param()).clamp(0.0, 1.0);
    paint_wheel(ui, surface, value, kind.springs_to_center(), &response);
    paint_label(ui, state, allocation.id, kind, label_rect, &response);
    response
}

fn update_wheel_value(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    kind: WheelKind,
    response: &egui::Response,
) {
    let param = kind.param();
    let edit_id = response.id.with("host-edit");
    if response.drag_started() {
        crate::editor::begin_edit(state, param);
        ui.data_mut(|data| data.insert_temp(edit_id, true));
    }
    if response.dragged() {
        let fine = ui.input(|input| input.modifiers.shift);
        let vertical_motion = response.drag_motion().y * if fine { 0.2 } else { 1.0 };
        let value = accumulate_drag(state.get_param(param), vertical_motion);
        state.set_param(param, f64::from(value));
    }

    let edit_active = ui.data(|data| data.get_temp::<bool>(edit_id).unwrap_or(false));
    if edit_active && (response.drag_stopped() || pointer_gesture_aborted(ui)) {
        if kind.springs_to_center() {
            state.set_param(param, kind.reset_value());
        }
        crate::editor::end_edit(state, param);
        ui.data_mut(|data| data.remove::<bool>(edit_id));
    } else if response.double_clicked() {
        crate::editor::begin_edit(state, param);
        state.set_param(param, kind.reset_value());
        crate::editor::end_edit(state, param);
    }
}

fn paint_wheel(
    ui: &egui::Ui,
    surface: egui::Rect,
    value: f32,
    bipolar: bool,
    response: &egui::Response,
) {
    let painter = ui.painter_at(surface);
    let palette = editor_theme::semantic();
    let rail = surface.shrink(editor_theme::space::XXS);
    let rounding = rail.width() * 0.5;
    let active = response.is_pointer_button_down_on() || response.dragged();
    let stroke_color = if response.hovered() || response.has_focus() || active {
        palette.masthead
    } else {
        palette.masthead.gamma_multiply(0.34)
    };
    painter.rect_filled(rail, rounding, palette.background.gamma_multiply(0.82));
    let marker_y = egui::lerp(rail.bottom()..=rail.top(), value.clamp(0.0, 1.0));
    let fill = if bipolar {
        egui::Rect::from_x_y_ranges(
            rail.x_range(),
            marker_y.min(rail.center().y)..=marker_y.max(rail.center().y),
        )
    } else {
        egui::Rect::from_min_max(egui::pos2(rail.left(), marker_y), rail.right_bottom())
    };
    painter.rect_filled(fill, rounding, palette.masthead.gamma_multiply(0.42));
    painter.line_segment(
        [rail.left_center(), rail.right_center()],
        egui::Stroke::new(
            editor_theme::shape::STROKE,
            palette
                .masthead
                .gamma_multiply(if bipolar { 0.52 } else { 0.22 }),
        ),
    );
    let thumb = egui::Rect::from_center_size(
        egui::pos2(rail.center().x, marker_y),
        egui::vec2(rail.width() * 0.72, (rail.width() * 0.20).max(2.0)),
    );
    painter.rect_filled(thumb, thumb.height() * 0.5, palette.masthead);
    painter.rect_stroke(
        rail,
        rounding,
        egui::Stroke::new(editor_theme::shape::STROKE, stroke_color),
        egui::StrokeKind::Inside,
    );
}

fn paint_label(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    allocation_id: egui::Id,
    kind: WheelKind,
    label_rect: Option<egui::Rect>,
    response: &egui::Response,
) {
    let Some(label_rect) = label_rect else {
        return;
    };
    let painter = ui.painter_at(label_rect);
    let label_color = if response.hovered() || response.has_focus() {
        editor_theme::semantic().masthead
    } else {
        editor_theme::semantic().text
    };
    if matches!(kind, WheelKind::Mod) {
        let jack_size = (label_rect.height() * 0.58).max(editor_theme::shape::FOCUS_STROKE * 2.0);
        let label_font = editor_theme::font::caption();
        let label_width = painter
            .layout_no_wrap(kind.label().to_owned(), label_font.clone(), label_color)
            .size()
            .x;
        let content_gap = editor_theme::space::XXS;
        let content_width = (jack_size + content_gap + label_width).min(label_rect.width());
        let content_left = label_rect.center().x - content_width * 0.5;
        let jack_rect = egui::Rect::from_center_size(
            egui::pos2(content_left + jack_size * 0.5, label_rect.center().y),
            egui::vec2(jack_size, jack_size),
        );
        let mut jack_response = ui.interact(
            label_rect,
            allocation_id.with("source"),
            egui::Sense::click(),
        );
        jack_response.rect = jack_rect;
        let _ = editor_modulation::source_handle_for(
            ui,
            state,
            ResolvedRouteSource::ModWheel,
            "MOD WHEEL",
            &jack_response,
        );
        painter.text(
            egui::pos2(
                jack_rect.right() + content_gap + label_width * 0.5,
                label_rect.center().y,
            ),
            egui::Align2::CENTER_CENTER,
            kind.label(),
            label_font,
            label_color,
        );
    } else {
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.label(),
            editor_theme::font::caption(),
            label_color,
        );
    }
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
    let wheel_bottom = label_rect
        .map_or(rect.bottom(), |label| label.top() - padding)
        .max(rect.top() + editor_theme::shape::STROKE);
    let wheel_area = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), wheel_bottom));
    let width = (wheel_area.width() * 0.84).max(editor_theme::space::LG);
    let surface = egui::Rect::from_center_size(
        wheel_area.center(),
        egui::vec2(width, wheel_area.height().max(editor_theme::shape::STROKE)),
    );
    (surface, label_rect)
}
