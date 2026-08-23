use std::f32::consts::{FRAC_PI_2, PI};

use truce_core::editor::{PluginContext, PluginContextReadF32};

use crate::editor_controls::{accumulate_drag, pointer_gesture_aborted};
use crate::editor_modulation;
use crate::modulators::routing::ResolvedRouteSource;
use crate::{KurvParams, P, editor_theme};

const RIB_COUNT: usize = 6;
const ROTATE_PERCENT: f32 = 0.8;
const BUFFER_RATIO: f32 = 0.05;
const LINE_WIDTH_RATIO: f32 = 0.165;
const FADE_RATIO: f32 = 0.12;
const WHEEL_ROUND_RATIO: f32 = 0.25;
const CONTAINER_ROUND_RATIO: f32 = 0.15;

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
    update_wheel_value(ui, state, kind, &response);

    let value = state.get_param(kind.param()).clamp(0.0, 1.0);
    let visuals = wheel_visuals(&response);
    paint_wheel(ui, surface, value, kind.springs_to_center(), visuals);
    paint_label(ui, state, allocation.id, kind, label_rect, visuals);
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

fn wheel_visuals(response: &egui::Response) -> editor_theme::ControlVisuals {
    let active = response.is_pointer_button_down_on() || response.dragged();
    editor_theme::control_visuals(
        response.enabled(),
        response.hovered(),
        active,
        response.has_focus(),
        editor_theme::semantic().primary,
    )
}

fn paint_wheel(
    ui: &egui::Ui,
    surface: egui::Rect,
    value: f32,
    bipolar: bool,
    visuals: editor_theme::ControlVisuals,
) {
    let painter = ui.painter_at(surface);
    let palette = editor_theme::semantic();
    let buffer = surface.width() * BUFFER_RATIO;
    let rounding = surface.width() * CONTAINER_ROUND_RATIO;
    let barrel = surface.shrink(buffer);
    painter.rect_filled(surface, rounding, palette.well.gamma_multiply(0.55));
    painter.rect_filled(barrel, rounding, visuals.fill.gamma_multiply(0.92));

    let t = ((1.0 - value.clamp(0.0, 1.0)) - 0.5) * ROTATE_PERCENT + 0.5;
    let spacing = 1.0 / RIB_COUNT as f32;
    let line_color = palette.text_muted.gamma_multiply(0.42);
    let fill_color = palette.control.gamma_multiply(0.92);
    let marker_color = visuals.indicator;
    let mut index = RIB_COUNT as i32;
    while t + spacing * (index as f32) >= 0.5 {
        paint_wheel_rib(
            &painter,
            barrel,
            t + spacing * (index as f32),
            if index == 0 { marker_color } else { line_color },
            fill_color,
        );
        index -= 1;
    }
    let mut index = -(RIB_COUNT as i32);
    while t + spacing * (index as f32) < 0.5 {
        paint_wheel_rib(
            &painter,
            barrel,
            t + spacing * (index as f32),
            if index == 0 { marker_color } else { line_color },
            fill_color,
        );
        index += 1;
    }

    painter.rect_stroke(barrel, rounding, visuals.stroke, egui::StrokeKind::Inside);
    if bipolar {
        painter.circle_filled(
            egui::pos2(barrel.right() - buffer, barrel.center().y),
            editor_theme::shape::STROKE,
            palette.text_muted.gamma_multiply(0.58),
        );
    }
}

fn paint_wheel_rib(
    painter: &egui::Painter,
    barrel: egui::Rect,
    y_percent: f32,
    line_color: egui::Color32,
    fill_color: egui::Color32,
) {
    let radians = PI * y_percent - FRAC_PI_2;
    if radians.abs() > PI * 0.6 {
        return;
    }
    let sin_value = radians.sin();
    let cos_value = radians.cos();
    let height = barrel.height();
    let y = barrel.center().y + sin_value * height * 0.45;
    let round_amount = sin_value.abs() * barrel.width() * WHEEL_ROUND_RATIO;
    let line_height = (cos_value * height * LINE_WIDTH_RATIO).max(0.0);
    let edge = (height * FADE_RATIO).max(1.0);
    let distance = (y - barrel.top()).min(barrel.bottom() - y);
    let alpha = (distance / edge).clamp(0.0, 1.0);
    let color = lerp_color(fill_color, line_color, alpha);
    let offset = (line_height + round_amount) * 0.5;
    let rib = egui::Rect::from_min_max(
        egui::pos2(barrel.left(), y - offset),
        egui::pos2(barrel.right(), y - offset + line_height + round_amount),
    );
    if rib.height() > 0.0 {
        painter.rect_filled(rib, round_amount, color);
    }
}

fn lerp_color(from: egui::Color32, to: egui::Color32, amount: f32) -> egui::Color32 {
    let amount = amount.clamp(0.0, 1.0);
    egui::Color32::from_rgba_unmultiplied(
        ((1.0 - amount) * f32::from(from.r()) + amount * f32::from(to.r())) as u8,
        ((1.0 - amount) * f32::from(from.g()) + amount * f32::from(to.g())) as u8,
        ((1.0 - amount) * f32::from(from.b()) + amount * f32::from(to.b())) as u8,
        ((1.0 - amount) * f32::from(from.a()) + amount * f32::from(to.a())) as u8,
    )
}

fn paint_label(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    allocation_id: egui::Id,
    kind: WheelKind,
    label_rect: Option<egui::Rect>,
    visuals: editor_theme::ControlVisuals,
) {
    let Some(label_rect) = label_rect else {
        return;
    };
    let painter = ui.painter_at(label_rect);
    if matches!(kind, WheelKind::Mod) {
        let jack_size = (label_rect.height() * 0.58).max(editor_theme::shape::FOCUS_STROKE * 2.0);
        let label_font = editor_theme::font::caption();
        let label_width = painter
            .layout_no_wrap(kind.label().to_owned(), label_font.clone(), visuals.label)
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
        painter.text(
            egui::pos2(
                jack_rect.right() + content_gap + label_width * 0.5,
                label_rect.center().y,
            ),
            egui::Align2::CENTER_CENTER,
            kind.label(),
            label_font,
            visuals.label,
        );
    } else {
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.label(),
            editor_theme::font::caption(),
            visuals.label,
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
    let width = (wheel_area.width() * 0.72)
        .min(editor_theme::space::LG * 2.0)
        .max(editor_theme::space::LG);
    let surface = egui::Rect::from_center_size(
        wheel_area.center(),
        egui::vec2(width, wheel_area.height().max(editor_theme::shape::STROKE)),
    );
    (surface, label_rect)
}
